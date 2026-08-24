//! d64 `delayed_events_live`: the append-only DEL2 lifecycle event part.
//!
//! d64 is a genuine hybrid. Its 20-byte base rows are mutable per-country
//! pending state, while its optional `DEL2` suffix is an independently produced,
//! append-only lifecycle ledger (`world_fold.rs:13683`-`:13816`, including the
//! delegated lifecycle-row fold).
// @world-fold-authority domain=64 functions=fold_delayed_events,fold_delayed_event_rows lines=13683-13816 sha256=123b847368be5ccd76a5eb867e5ea30e106b0bedd96c0bf06a34f9a9f6dc0e71 closure=8e631f6adedd4e91d01d5bc44c8a22fb23dbf296801fb8f8b31222941801445d
//! The state table is an ordered occurrence list whose physical vector order
//! must be preserved. The ordered-occurrence codec can represent that shape, but
//! the d64 pending-state collection remains independently unported and
//! unevidenced. This module therefore implements only the event-continuity contract event part; it
//! neither invents a key nor treats codec availability as migration evidence.
//!
//! # DEL2 grammar and call order
//!
//! `fold_delayed_events` first checks the 20-byte base header, requires
//! `observed_rows == row_count`, computes `20 + row_count * 20` with checked
//! arithmetic, and requires the payload to reach that boundary
//! (`world_fold.rs::fold_delayed_events`). If bytes remain, it parses the whole DEL2
//! trailer before consulting the table status (`world_fold.rs::fold_delayed_events`). Only then does
//! the table failure gate run; the independently drained ledger is settled on
//! every table path (`world_fold.rs::fold_delayed_events`). [`split_payload`], [`parse_trailer`] and
//! [`fold_payload`] preserve that order.
//!
//! Header and row checks are transcribed from `world_fold.rs::parse_delayed_event_lifecycle`.
//! Their shipping original lives in `recorder_host`, which this library may not
//! depend on. `bench_real_d64` therefore checks every reachable constant against
//! `compact_contract`, runs the transcribed status-independent payload authority
//! on every d64 DATA record, and makes every real payload reproduce its length
//! from the strides. This is the recipe's weaker empirical stand-in, not a
//! machine-level dual decoder.
//!
//! # Mapping decisions
//!
//! - `sequence` is not a field. The fold requires row `sequence ==
//!   first_sequence + index` (`world_fold.rs::parse_delayed_event_lifecycle`), so event-continuity contract derives it
//!   from event position and the after-frontier.
//!   [`event_batch_from_lifecycle_payload`] enforces the header bounds and every
//!   row's expected position before omitting it.
//! - `reason` is the closed four-way transform at `world_fold.rs::parse_delayed_event_lifecycle`, represented
//!   as a four-cardinality enum (wire codes 0..=3).
//! - `due_after` is not a field. On the only arm where it exists the fold
//!   requires `due_after == due_before - 1` (`world_fold.rs::parse_delayed_event_lifecycle`); on all other arms
//!   it requires the zero-packed absence word (`world_fold.rs::parse_delayed_event_lifecycle`). It is therefore
//!   a total function of carried `reason` and `due_before`, and carrying a second
//!   copy would admit contradictory bytes. [`record_from_lifecycle_row`] refuses
//!   every contradictory public row before omitting it.
//! - owner and originator country ids are `ContentId`, following the recipe's
//!   d20 country-id precedent. `event_id` is also `ContentId`: its meaning is a
//!   mod-defined event definition and needs the content manifest across mod
//!   sets, unlike a runtime instance id. All three currently use the shipping
//!   raw numeric id as the content-identity contract stand-in; their resolver retrofit remains open.
//! - `scope_random` is the full `u64` seed as `Unsigned`; no precision or bit is
//!   lost. It is neither a quantity nor a content-manifest identity.
//! - `game_hour_raw` is a signed `i32`; `i32::MIN` is refused (`:9734`,
//!   `:9747`). It is the event's occurrence clock, distinct from the carrying
//!   record's observation hour.
//!
//! The ContentId retrofit sweep is therefore: `owner_tag_db_id`, `event_id`,
//! and `originator_tag_db_id`. None is a key because event-continuity contract event parts have no key.

use crate::events::EventBatch;
use crate::record::Record;
use crate::schema::{EventPart, Field, Kind, Schema};
use crate::value::Value;

pub const DOMAIN: u32 = 64;
/// Collection id 1 remains reserved for the unported pending-state table.
pub const LIFECYCLE: u32 = 2;

pub mod field {
    pub const GAME_HOUR_RAW: u32 = 1;
    pub const REASON: u32 = 2;
    pub const OWNER_TAG_DB_ID: u32 = 3;
    pub const EVENT_ID: u32 = 4;
    pub const ORIGINATOR_TAG_DB_ID: u32 = 5;
    pub const SCOPE_RANDOM: u32 = 6;
    pub const DUE_BEFORE: u32 = 7;
}

pub mod frontier {
    pub const HOOK_EPOCH: u32 = 1;
    pub const SEQUENCE: u32 = 2;
}

pub const BASE_HEADER_LEN: usize = 20;
pub const BASE_ROW_LEN: usize = 20;
pub const BASE_FLAG_DUE_REMAINING_HOURS: u32 = 1 << 0;
pub const BASE_FLAG_ORIGINATOR: u32 = 1 << 1;
pub const BASE_KNOWN_FLAGS: u32 = BASE_FLAG_DUE_REMAINING_HOURS | BASE_FLAG_ORIGINATOR;

pub const TRAILER_MAGIC: &[u8; 4] = b"DEL2";
pub const TRAILER_VERSION: u16 = 1;
pub const TRAILER_HEADER_LEN: usize = 48;
pub const TRAILER_ROW_LEN: usize = 48;
pub const TRAILER_FLAG_COMPLETE: u32 = 1 << 0;
pub const ROW_FLAG_DUE_AFTER: u32 = 1 << 0;
pub const MAX_DUE_REMAINING_HOURS: u32 = 500_000;

/// d64 has no lane-local aliases and no expected-partial tolerance, so
/// `handle_compact_failure` uses the global mask (`world_fold.rs:9548`-`:9549`;
/// `compact_contract/src/lib.rs:148`-`:163`, `:224`-`:233`). The measurement bin proves this
/// copy against the shared predicate over all assigned status words.
pub const FAILURE_STATUS_MASK: u32 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 9)
    | (1 << 12);

pub fn schema() -> Schema {
    Schema {
        domain: DOMAIN,
        cadence_log2: 0,
        // The generic ordered-occurrence body exists, but this domain's
        // pending-state mapper and evidence do not. Its id remains reserved.
        collections: Vec::new(),
        event_part: Some(EventPart {
            collection_id: LIFECYCLE,
            fields: vec![
                Field::new(field::GAME_HOUR_RAW, Kind::Signed),
                Field::new(field::REASON, Kind::Enum).enum_card(4),
                Field::new(field::OWNER_TAG_DB_ID, Kind::ContentId),
                Field::new(field::EVENT_ID, Kind::ContentId),
                Field::new(field::ORIGINATOR_TAG_DB_ID, Kind::ContentId),
                Field::new(field::SCOPE_RANDOM, Kind::Unsigned),
                Field::new(field::DUE_BEFORE, Kind::Unsigned),
            ],
            frontier_component_ids: vec![frontier::HOOK_EPOCH, frontier::SEQUENCE],
            sequence_index: 1,
        }),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum LifecycleReason {
    RetainedDecrement = 1,
    Executed = 2,
    FailedTriggerConsumed = 3,
    DefinitionCancelled = 4,
}

impl LifecycleReason {
    pub fn enum_code(self) -> u32 {
        self as u32 - 1
    }

    fn from_raw(raw: u32) -> Option<Self> {
        match raw {
            1 => Some(Self::RetainedDecrement),
            2 => Some(Self::Executed),
            3 => Some(Self::FailedTriggerConsumed),
            4 => Some(Self::DefinitionCancelled),
            _ => None,
        }
    }

    fn from_enum_code(code: u32) -> Option<Self> {
        Self::from_raw(code.checked_add(1)?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleRow {
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub reason: LifecycleReason,
    pub owner_tag_db_id: u32,
    pub event_id: u32,
    pub originator_tag_db_id: u32,
    pub scope_random: u64,
    pub due_before: u32,
    pub due_after: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecyclePayload {
    pub complete: bool,
    pub hook_epoch: u64,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub events: Vec<LifecycleRow>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub observed_rows: u32,
    pub status_flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct PayloadParts<'a> {
    pub header: PayloadHeader,
    pub state_rows: &'a [u8],
    pub trailer: Option<&'a [u8]>,
}

/// The fold's pre-status structural prefix, in its own call order
/// (`world_fold.rs::fold_delayed_events`).
pub fn split_payload(payload: &[u8]) -> Result<PayloadParts<'_>, String> {
    if payload.len() < BASE_HEADER_LEN {
        return Err(format!(
            "delayed_events header is {} byte(s), expected at least {BASE_HEADER_LEN}",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64_at(payload, 0),
        row_count: u32_at(payload, 8),
        observed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
    };
    let rows_bytes = (header.row_count as usize)
        .checked_mul(BASE_ROW_LEN)
        .ok_or_else(|| "delayed_events row length overflow".to_string())?;
    let rows_end = BASE_HEADER_LEN
        .checked_add(rows_bytes)
        .ok_or_else(|| "delayed_events row length overflow".to_string())?;
    if header.observed_rows != header.row_count {
        return Err("delayed_events observed/packed row count mismatch".into());
    }
    if payload.len() < rows_end {
        return Err(format!(
            "delayed_events len {} is shorter than {rows_end}",
            payload.len()
        ));
    }
    Ok(PayloadParts {
        header,
        state_rows: &payload[BASE_HEADER_LEN..rows_end],
        trailer: (payload.len() != rows_end).then_some(&payload[rows_end..]),
    })
}

/// Parse DEL2 exactly as `world_fold.rs::parse_delayed_event_lifecycle`: header structure,
/// count/sequence arithmetic, then rows in wire order.
pub fn parse_trailer(payload: &[u8]) -> Result<LifecyclePayload, String> {
    if payload.len() < TRAILER_HEADER_LEN {
        return Err(format!(
            "delayed_event_lifecycle header is {} byte(s), expected at least {TRAILER_HEADER_LEN}",
            payload.len()
        ));
    }
    if payload.get(0..4) != Some(TRAILER_MAGIC.as_slice())
        || u16_at(payload, 4) != TRAILER_VERSION
        || u16_at(payload, 6) as usize != TRAILER_HEADER_LEN
        || u16_at(payload, 8) as usize != TRAILER_ROW_LEN
        || u16_at(payload, 10) != 0
        || u32_at(payload, 44) != 0
    {
        return Err("delayed_event_lifecycle header rejected".into());
    }
    let flags = u32_at(payload, 12);
    if flags & !TRAILER_FLAG_COMPLETE != 0 {
        return Err("delayed_event_lifecycle unknown header flags".into());
    }
    let count = u32_at(payload, 16) as usize;
    let hook_epoch = u64_at(payload, 20);
    let first_sequence = u64_at(payload, 28);
    let last_sequence = u64_at(payload, 36);
    let expected = count
        .checked_mul(TRAILER_ROW_LEN)
        .and_then(|rows| TRAILER_HEADER_LEN.checked_add(rows))
        .ok_or_else(|| "delayed_event_lifecycle length overflow".to_string())?;
    if payload.len() != expected
        || hook_epoch == 0
        || (count == 0 && (first_sequence != 0 || last_sequence != 0))
        || (count != 0
            && last_sequence
                .checked_sub(first_sequence)
                .and_then(|delta| delta.checked_add(1))
                != Some(count as u64))
    {
        return Err("delayed_event_lifecycle count/sequence header rejected".into());
    }

    let mut events = Vec::new();
    events
        .try_reserve_exact(count)
        .map_err(|_| "delayed_event_lifecycle count exceeds allocation bounds".to_string())?;
    for index in 0..count {
        let base = TRAILER_HEADER_LEN + index * TRAILER_ROW_LEN;
        let sequence = u64_at(payload, base);
        let game_hour_raw = i32_at(payload, base + 8);
        let reason_raw = u32_at(payload, base + 12);
        let owner_tag_db_id = u32_at(payload, base + 16);
        let event_id = u32_at(payload, base + 20);
        let originator_tag_db_id = u32_at(payload, base + 24);
        let row_flags = u32_at(payload, base + 28);
        let scope_random = u64_at(payload, base + 32);
        let due_before = u32_at(payload, base + 40);
        let due_after_raw = u32_at(payload, base + 44);
        let expected_sequence = first_sequence
            .checked_add(index as u64)
            .ok_or_else(|| "delayed_event_lifecycle sequence overflow".to_string())?;
        if sequence != expected_sequence
            || game_hour_raw == i32::MIN
            || owner_tag_db_id == 0
            || originator_tag_db_id == 0
            || !(100_000..=100_099_999).contains(&event_id)
            || row_flags & !ROW_FLAG_DUE_AFTER != 0
            || due_before > MAX_DUE_REMAINING_HOURS
        {
            return Err(format!("delayed_event_lifecycle row {index} rejected"));
        }
        let has_due_after = row_flags & ROW_FLAG_DUE_AFTER != 0;
        let (reason, due_after) = match (LifecycleReason::from_raw(reason_raw), has_due_after) {
            (Some(LifecycleReason::RetainedDecrement), true)
                if due_before.checked_sub(1) == Some(due_after_raw)
                    && due_after_raw <= MAX_DUE_REMAINING_HOURS =>
            {
                (LifecycleReason::RetainedDecrement, Some(due_after_raw))
            }
            (Some(reason @ LifecycleReason::Executed), false)
            | (Some(reason @ LifecycleReason::FailedTriggerConsumed), false)
            | (Some(reason @ LifecycleReason::DefinitionCancelled), false)
                if due_after_raw == 0 =>
            {
                (reason, None)
            }
            _ => {
                return Err(format!(
                    "delayed_event_lifecycle row {index} reason/due transform rejected"
                ))
            }
        };
        events.push(LifecycleRow {
            sequence,
            game_hour_raw,
            reason,
            owner_tag_db_id,
            event_id,
            originator_tag_db_id,
            scope_random,
            due_before,
            due_after,
        });
    }
    Ok(LifecyclePayload {
        complete: flags & TRAILER_FLAG_COMPLETE != 0,
        hook_epoch,
        first_sequence,
        last_sequence,
        events,
    })
}

/// The shipping status-independent payload authority at
/// `domain_data_authority.rs:12083`-`:12298`, transcribed for the measurement
/// binary. This is intentionally distinct from [`split_payload`]: the authority
/// requires `observed_rows <= row_count`, while the fold requires equality.
pub fn check_authority_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() < BASE_HEADER_LEN {
        return Err("delayed_events payload shorter than authority header".into());
    }
    let row_count = u32_at(payload, 8) as usize;
    let changed_rows = u32_at(payload, 12) as usize;
    let rows_end = row_count
        .checked_mul(BASE_ROW_LEN)
        .and_then(|rows| BASE_HEADER_LEN.checked_add(rows))
        .ok_or_else(|| "delayed_events authority row length overflow".to_string())?;
    if payload.len() < rows_end {
        return Err(format!(
            "delayed_events authority len {} is shorter than {rows_end}",
            payload.len()
        ));
    }
    if payload.len() != rows_end {
        parse_trailer(&payload[rows_end..])?;
    }
    if changed_rows > row_count {
        return Err(format!(
            "delayed_events changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    for index in 0..row_count {
        let base = BASE_HEADER_LEN + index * BASE_ROW_LEN;
        let owner = u32_at(payload, base);
        let event_id = u32_at(payload, base + 4);
        let flags = u32_at(payload, base + 8);
        let originator = u32_at(payload, base + 12);
        let due = u32_at(payload, base + 16);
        if owner == 0 {
            return Err(format!("delayed_events row {index} has zero owner"));
        }
        if event_id == 0 {
            return Err(format!("delayed_events row {index} has zero event_id"));
        }
        if flags & !BASE_KNOWN_FLAGS != 0 {
            return Err(format!("delayed_events row {index} has unknown flags"));
        }
        let has_due = flags & BASE_FLAG_DUE_REMAINING_HOURS != 0;
        let has_originator = flags & BASE_FLAG_ORIGINATOR != 0;
        if !has_due && due != 0 {
            return Err(format!("delayed_events row {index} due without flag"));
        }
        if has_due && due > MAX_DUE_REMAINING_HOURS {
            return Err(format!("delayed_events row {index} due out of range"));
        }
        if !has_originator && originator != 0 {
            return Err(format!(
                "delayed_events row {index} originator without flag"
            ));
        }
    }
    Ok(())
}

/// Validate and map one DEL2 row to its full record-state contract event record. `frame_tick` is
/// intentionally unused: no d64 lifecycle field is clock-derived.
fn require_canonical_event_part(part: &EventPart) -> Result<(), String> {
    let canonical = schema()
        .event_part
        .ok_or_else(|| "d64 schema has no lifecycle event part".to_string())?;
    if part != &canonical {
        return Err("not the canonical d64 lifecycle event part".into());
    }
    Ok(())
}

pub fn record_from_lifecycle_row(
    part: &EventPart,
    row: &LifecycleRow,
    _frame_tick: u64,
) -> Result<Record, String> {
    require_canonical_event_part(part)?;
    validate_lifecycle_row_for_mapping(row)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(part.fields.len())
        .map_err(|_| "d64 event field count exceeds allocation bounds".to_string())?;
    for field in &part.fields {
        let value = match field.field_id {
            field::GAME_HOUR_RAW => Value::Signed(row.game_hour_raw as i64),
            field::REASON => Value::Enum(row.reason.enum_code()),
            field::OWNER_TAG_DB_ID => Value::ContentId(row.owner_tag_db_id as u64),
            field::EVENT_ID => Value::ContentId(row.event_id as u64),
            field::ORIGINATOR_TAG_DB_ID => Value::ContentId(row.originator_tag_db_id as u64),
            field::SCOPE_RANDOM => Value::Unsigned(row.scope_random),
            field::DUE_BEFORE => Value::Unsigned(row.due_before as u64),
            other => return Err(format!("d64 has no mapping for event field id {other}")),
        };
        values.push(value);
    }
    let record = Record::new(values);
    let collection = crate::Collection {
        collection_id: part.collection_id,
        shape: crate::Shape::OrderedOccurrenceList,
        fields: part.fields.clone(),
    };
    record
        .validate(&collection)
        .map_err(|error| format!("d64 lifecycle record does not match schema: {error:?}"))?;
    Ok(record)
}

fn validate_lifecycle_row_for_mapping(row: &LifecycleRow) -> Result<(), String> {
    if row.game_hour_raw == i32::MIN {
        return Err("d64 lifecycle mapper refuses i32::MIN game_hour_raw".into());
    }
    if row.owner_tag_db_id == 0 {
        return Err("d64 lifecycle mapper refuses zero owner_tag_db_id".into());
    }
    if row.originator_tag_db_id == 0 {
        return Err("d64 lifecycle mapper refuses zero originator_tag_db_id".into());
    }
    if !(100_000..=100_099_999).contains(&row.event_id) {
        return Err("d64 lifecycle mapper refuses out-of-range event_id".into());
    }
    if row.due_before > MAX_DUE_REMAINING_HOURS {
        return Err("d64 lifecycle mapper refuses out-of-range due_before".into());
    }
    let expected_due_after = match row.reason {
        LifecycleReason::RetainedDecrement => row
            .due_before
            .checked_sub(1)
            .map(Some)
            .ok_or_else(|| "d64 retained event has zero due_before".to_string())?,
        LifecycleReason::Executed
        | LifecycleReason::FailedTriggerConsumed
        | LifecycleReason::DefinitionCancelled => None,
    };
    if row.due_after != expected_due_after {
        return Err(format!(
            "d64 lifecycle mapper refuses {:?} with due_before {} and due_after {:?}; expected {:?}",
            row.reason, row.due_before, row.due_after, expected_due_after
        ));
    }
    Ok(())
}

/// Validate the payload-level facts omitted from each record, then build the
/// event-continuity contract batch. `sequence` is enforced against `(first_sequence, position)` here
/// and represented only by the after-frontier; `due_after` is enforced by the
/// row mapper and represented only by `(reason, due_before)`.
pub fn event_batch_from_lifecycle_payload(
    part: &EventPart,
    lifecycle: &LifecyclePayload,
    frame_tick: u64,
) -> Result<EventBatch, String> {
    require_canonical_event_part(part)?;
    if !lifecycle.complete {
        return Err("d64 incomplete lifecycle observation requires an event-part gap".into());
    }
    if lifecycle.hook_epoch == 0 {
        return Err("d64 lifecycle batch has zero hook_epoch".into());
    }
    let count = lifecycle.events.len();
    if count == 0 {
        if lifecycle.first_sequence != 0 || lifecycle.last_sequence != 0 {
            return Err("d64 empty lifecycle batch has non-zero sequence bounds".into());
        }
        return Err("d64 empty lifecycle observation has no event batch".into());
    } else {
        let count_u64 =
            u64::try_from(count).map_err(|_| "d64 lifecycle event count exceeds u64")?;
        if lifecycle
            .last_sequence
            .checked_sub(lifecycle.first_sequence)
            .and_then(|delta| delta.checked_add(1))
            != Some(count_u64)
        {
            return Err("d64 lifecycle batch sequence bounds do not match event count".into());
        }
        if lifecycle.last_sequence < count_u64 {
            return Err("d64 lifecycle batch cannot derive positive identities".into());
        }
        for (index, row) in lifecycle.events.iter().enumerate() {
            let expected = lifecycle
                .first_sequence
                .checked_add(index as u64)
                .ok_or_else(|| "d64 lifecycle sequence overflow".to_string())?;
            if row.sequence != expected {
                return Err(format!(
                    "d64 lifecycle row {index} sequence {} != expected {expected}",
                    row.sequence
                ));
            }
        }
    }
    let records = lifecycle
        .events
        .iter()
        .map(|row| record_from_lifecycle_row(part, row, frame_tick))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(EventBatch::new(
        records,
        vec![lifecycle.hook_epoch, lifecycle.last_sequence],
    ))
}

/// Reconstruct the fold's rows from a decoded event-continuity contract batch, including the sequence
/// and `due_after` values that are derived rather than carried.
pub fn rows_from_event_batch(
    part: &EventPart,
    batch: &EventBatch,
) -> Result<Vec<LifecycleRow>, String> {
    require_canonical_event_part(part)?;
    if batch.records.is_empty() {
        return Err("d64 event batch is empty".into());
    }
    if batch.frontier.len() != part.frontier_component_ids.len() {
        return Err("d64 event frontier arity does not match schema".into());
    }
    if batch.frontier[0] == 0 {
        return Err("d64 decoded lifecycle frontier has zero hook_epoch".into());
    }
    let mut rows = Vec::new();
    rows.try_reserve_exact(batch.records.len())
        .map_err(|_| "d64 event count exceeds allocation bounds".to_string())?;
    for (index, record) in batch.records.iter().enumerate() {
        if record.values.len() != part.fields.len() {
            return Err("d64 decoded record arity does not match event schema".into());
        }
        let mut game_hour_raw = None;
        let mut reason = None;
        let mut owner = None;
        let mut event_id = None;
        let mut originator = None;
        let mut scope_random = None;
        let mut due_before = None;
        for (field, value) in part.fields.iter().zip(&record.values) {
            match (field.field_id, *value) {
                (field::GAME_HOUR_RAW, Value::Signed(value)) => {
                    game_hour_raw =
                        Some(i32::try_from(value).map_err(|_| {
                            "d64 decoded game_hour_raw does not fit i32".to_string()
                        })?)
                }
                (field::REASON, Value::Enum(value)) => {
                    reason = LifecycleReason::from_enum_code(value)
                }
                (field::OWNER_TAG_DB_ID, Value::ContentId(value)) => {
                    owner =
                        Some(u32::try_from(value).map_err(|_| {
                            "d64 decoded owner_tag_db_id does not fit u32".to_string()
                        })?)
                }
                (field::EVENT_ID, Value::ContentId(value)) => {
                    event_id = Some(
                        u32::try_from(value)
                            .map_err(|_| "d64 decoded event_id does not fit u32".to_string())?,
                    )
                }
                (field::ORIGINATOR_TAG_DB_ID, Value::ContentId(value)) => {
                    originator = Some(u32::try_from(value).map_err(|_| {
                        "d64 decoded originator_tag_db_id does not fit u32".to_string()
                    })?)
                }
                (field::SCOPE_RANDOM, Value::Unsigned(value)) => scope_random = Some(value),
                (field::DUE_BEFORE, Value::Unsigned(value)) => {
                    due_before = Some(
                        u32::try_from(value)
                            .map_err(|_| "d64 decoded due_before does not fit u32".to_string())?,
                    )
                }
                _ => return Err("d64 decoded value does not match its event field".into()),
            }
        }
        let reason = reason.ok_or_else(|| "d64 decoded reason is missing".to_string())?;
        let due_before =
            due_before.ok_or_else(|| "d64 decoded due_before is missing".to_string())?;
        let due_after = match reason {
            LifecycleReason::RetainedDecrement => Some(
                due_before
                    .checked_sub(1)
                    .ok_or_else(|| "d64 retained event has zero due_before".to_string())?,
            ),
            _ => None,
        };
        let sequence = batch
            .sequence_at(part, index)
            .map_err(|error| format!("d64 cannot derive event sequence: {error:?}"))?;
        let row = LifecycleRow {
            sequence,
            game_hour_raw: game_hour_raw
                .ok_or_else(|| "d64 decoded game_hour_raw is missing".to_string())?,
            reason,
            owner_tag_db_id: owner.ok_or_else(|| "d64 decoded owner is missing".to_string())?,
            event_id: event_id.ok_or_else(|| "d64 decoded event id is missing".to_string())?,
            originator_tag_db_id: originator
                .ok_or_else(|| "d64 decoded originator is missing".to_string())?,
            scope_random: scope_random
                .ok_or_else(|| "d64 decoded scope_random is missing".to_string())?,
            due_before,
            due_after,
        };
        validate_lifecycle_row_for_mapping(&row)?;
        rows.push(row);
    }
    Ok(rows)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TablePart {
    /// Failure status: production drops only the table half.
    StoppedUnavailable,
    /// The table parsed far enough to be present, but collection-shape contract support is absent.
    StoppedPresent { row_count: u32 },
    /// The clean-status table refused structurally. Production retains the
    /// prior table state but still publishes an independently valid ledger.
    Refused { error: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LedgerPart {
    /// No drain occurred; on a table-failure path the producer retains queued
    /// events for a later contiguous drain (`world_fold.rs:9634`-`:9641`).
    NoObservation,
    Published(LifecyclePayload),
    /// Production publishes null and keeps its sticky poison set.
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldOutcome {
    pub table: TablePart,
    pub ledger: LedgerPart,
}

/// Production's cross-hour d64 lifecycle state
/// (`world_fold.rs::settle_delayed_event_lifecycle`). The sticky poison is intentionally mirrored,
/// even though event-continuity contract recovers after a recorded gap; that divergence is measured and
/// reported rather than silently "fixed" at this boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct State {
    lifecycle_seen: bool,
    history_failed: bool,
    hook_epoch: Option<u64>,
    last_sequence: Option<u64>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn history_failed(&self) -> bool {
        self.history_failed
    }

    pub fn hook_epoch(&self) -> Option<u64> {
        self.hook_epoch
    }

    pub fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }
}

/// Mirror `fold_delayed_events` and `settle_delayed_event_lifecycle`.
///
/// A trailer parse error mutates `history_failed` before returning `Err`, as the
/// fold does (`world_fold.rs:9521`-`:9527`). A table-row refusal is carried in
/// [`TablePart::Refused`] beside the already-settled ledger: production settles
/// at `world_fold.rs::fold_delayed_events`, records the table error at `world_fold.rs::apply_domain_data`, and emits
/// the ledger independently at `world_fold.rs::build_snapshot`.
pub fn fold_payload(state: &mut State, payload: &[u8]) -> Result<FoldOutcome, String> {
    let parts = split_payload(payload)?;
    let lifecycle = match parts.trailer {
        Some(trailer) => {
            state.lifecycle_seen = true;
            match parse_trailer(trailer) {
                Ok(lifecycle) => Some(lifecycle),
                Err(error) => {
                    state.history_failed = true;
                    return Err(error);
                }
            }
        }
        None => None,
    };

    let table_failed = parts.header.status_flags & FAILURE_STATUS_MASK != 0;
    let table_result = if table_failed {
        Ok(TablePart::StoppedUnavailable)
    } else {
        // Validate the table rows only after the status gate, in the fold's
        // order. Preserve its Result until after ledger settlement: production
        // settles the independent trailer even when a clean table row refuses
        // (`world_fold.rs::fold_delayed_events`).
        validate_state_rows(parts.state_rows, parts.header.row_count).map(|()| {
            TablePart::StoppedPresent {
                row_count: parts.header.row_count,
            }
        })
    };

    let ledger = if let Some(lifecycle) = lifecycle {
        let continuity_ok = lifecycle.complete
            && !state.history_failed
            && match state.hook_epoch {
                None => lifecycle.first_sequence == 0 || lifecycle.first_sequence == 1,
                Some(epoch) if epoch == lifecycle.hook_epoch => {
                    match (state.last_sequence, lifecycle.first_sequence) {
                        (_, 0) => lifecycle.events.is_empty(),
                        (Some(last), first) => last.checked_add(1) == Some(first),
                        (None, first) => first == 1,
                    }
                }
                Some(_) => false,
            };
        if !continuity_ok {
            state.history_failed = true;
            LedgerPart::Unknown
        } else {
            state.hook_epoch = Some(lifecycle.hook_epoch);
            if lifecycle.last_sequence != 0 {
                state.last_sequence = Some(lifecycle.last_sequence);
            }
            LedgerPart::Published(lifecycle)
        }
    } else if state.lifecycle_seen && !table_failed {
        state.history_failed = true;
        LedgerPart::Unknown
    } else {
        LedgerPart::NoObservation
    };
    let table = match table_result {
        Ok(table) => table,
        Err(error) => TablePart::Refused { error },
    };
    Ok(FoldOutcome { table, ledger })
}

fn validate_state_rows(rows: &[u8], row_count: u32) -> Result<(), String> {
    for index in 0..row_count as usize {
        let base = index * BASE_ROW_LEN;
        let row = &rows[base..base + BASE_ROW_LEN];
        let owner = u32_at(row, 0);
        let event_id = u32_at(row, 4);
        let flags = u32_at(row, 8);
        let originator = u32_at(row, 12);
        let due = u32_at(row, 16);
        if owner == 0 {
            return Err("delayed_events row with zero owner tag_db_id".into());
        }
        if event_id == 0 {
            return Err("delayed_events row with zero event_id".into());
        }
        if flags & !BASE_KNOWN_FLAGS != 0 {
            return Err(format!(
                "delayed_events row {index} unknown flags {flags:#x}"
            ));
        }
        let has_due = flags & BASE_FLAG_DUE_REMAINING_HOURS != 0;
        let has_originator = flags & BASE_FLAG_ORIGINATOR != 0;
        if !has_due && due != 0 {
            return Err(format!(
                "delayed_events row {index} due_remaining_hours without flag"
            ));
        }
        if has_due && due > MAX_DUE_REMAINING_HOURS {
            return Err(format!(
                "delayed_events row {index} due_remaining_hours out of range"
            ));
        }
        if !has_originator && originator != 0 {
            return Err(format!(
                "delayed_events row {index} originator without flag"
            ));
        }
        if has_originator && originator == 0 {
            return Err(format!("delayed_events row {index} has zero originator"));
        }
    }
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_event_body, encode_event_body, TickGrid};

    fn event_part() -> EventPart {
        schema().event_part.expect("d64 has an event part")
    }

    fn row(sequence: u64, reason: LifecycleReason) -> LifecycleRow {
        let (due_before, due_after) = match reason {
            LifecycleReason::RetainedDecrement => (10, Some(9)),
            _ => (10, None),
        };
        LifecycleRow {
            sequence,
            game_hour_raw: 60_000_000,
            reason,
            owner_tag_db_id: 1,
            event_id: 100_001,
            originator_tag_db_id: 2,
            scope_random: 0x8877_6655_4433_2211,
            due_before,
            due_after,
        }
    }

    fn trailer(complete: bool, epoch: u64, first: u64, rows: &[LifecycleRow]) -> Vec<u8> {
        let last = rows.last().map(|r| r.sequence).unwrap_or(0);
        let mut out = Vec::new();
        out.extend_from_slice(TRAILER_MAGIC);
        out.extend_from_slice(&TRAILER_VERSION.to_le_bytes());
        out.extend_from_slice(&(TRAILER_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&(TRAILER_ROW_LEN as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(if complete { TRAILER_FLAG_COMPLETE } else { 0 }).to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&epoch.to_le_bytes());
        out.extend_from_slice(&(if rows.is_empty() { 0 } else { first }).to_le_bytes());
        out.extend_from_slice(&last.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for row in rows {
            out.extend_from_slice(&row.sequence.to_le_bytes());
            out.extend_from_slice(&row.game_hour_raw.to_le_bytes());
            out.extend_from_slice(&(row.reason as u32).to_le_bytes());
            out.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
            out.extend_from_slice(&row.event_id.to_le_bytes());
            out.extend_from_slice(&row.originator_tag_db_id.to_le_bytes());
            out.extend_from_slice(
                &(if row.due_after.is_some() {
                    ROW_FLAG_DUE_AFTER
                } else {
                    0
                })
                .to_le_bytes(),
            );
            out.extend_from_slice(&row.scope_random.to_le_bytes());
            out.extend_from_slice(&row.due_before.to_le_bytes());
            out.extend_from_slice(&row.due_after.unwrap_or(0).to_le_bytes());
        }
        out
    }

    fn payload(status: u32, trailer: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&123u64.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&status.to_le_bytes());
        if let Some(trailer) = trailer {
            out.extend_from_slice(trailer);
        }
        out
    }

    #[test]
    fn schema_is_valid_and_reserves_the_stopped_state_id() {
        schema()
            .validate(TickGrid::HOURLY.log2())
            .expect("d64 event schema is well formed");
        let schema = schema();
        assert!(
            schema.collections.is_empty(),
            "pending-state collection remains independently unported"
        );
        let part = schema.event_part.unwrap();
        assert_eq!(
            part.collection_id, 2,
            "id 1 stays reserved for pending state"
        );
        assert_eq!(part.frontier_component_ids, vec![1, 2]);
        assert_eq!(part.sequence_index, 1);
    }

    #[test]
    fn event_fields_have_no_keys_and_exactly_the_declared_kinds() {
        let part = event_part();
        assert!(part.fields.iter().all(|field| !field.is_key()));
        assert_eq!(
            part.fields
                .iter()
                .map(|field| (field.field_id, field.kind))
                .collect::<Vec<_>>(),
            vec![
                (field::GAME_HOUR_RAW, Kind::Signed),
                (field::REASON, Kind::Enum),
                (field::OWNER_TAG_DB_ID, Kind::ContentId),
                (field::EVENT_ID, Kind::ContentId),
                (field::ORIGINATOR_TAG_DB_ID, Kind::ContentId),
                (field::SCOPE_RANDOM, Kind::Unsigned),
                (field::DUE_BEFORE, Kind::Unsigned),
            ]
        );
        assert_eq!(part.fields[1].enum_card, Some(4));
        let content_ids: Vec<u32> = part
            .fields
            .iter()
            .filter(|field| field.kind == Kind::ContentId)
            .map(|field| field.field_id)
            .collect();
        assert_eq!(
            content_ids,
            vec![
                field::OWNER_TAG_DB_ID,
                field::EVENT_ID,
                field::ORIGINATOR_TAG_DB_ID
            ]
        );
    }

    #[test]
    fn mapper_states_are_all_concrete_and_zero_stays_a_value() {
        // Recipe step 8's states test: this event schema declares no optional,
        // nullable, unknown-capable or legacy capability, so Absent/Null/Unknown
        // are inapplicable. Exercise full values and every reachable zero.
        let part = event_part();
        let mut input = row(1, LifecycleReason::Executed);
        input.game_hour_raw = 0;
        input.scope_random = 0;
        input.due_before = 0;
        let record = record_from_lifecycle_row(&part, &input, 999).unwrap();
        assert_eq!(record.values[0], Value::Signed(0));
        assert_eq!(record.values[1], Value::Enum(1));
        assert_eq!(record.values[5], Value::Unsigned(0));
        assert_eq!(record.values[6], Value::Unsigned(0));
        assert!(record
            .values
            .iter()
            .all(|value| !matches!(value, Value::Absent | Value::Null | Value::Unknown)));
        assert!(part
            .fields
            .iter()
            .all(|field| !field.optional && !field.nullable && !field.unknown_capable));
    }

    #[test]
    fn full_event_row_roundtrips_byte_identically_and_recovers_derived_values() {
        let part = event_part();
        let rows = vec![
            row(41, LifecycleReason::RetainedDecrement),
            row(42, LifecycleReason::DefinitionCancelled),
        ];
        let batch = event_batch_from_lifecycle_payload(
            &part,
            &LifecyclePayload {
                complete: true,
                hook_epoch: 9,
                first_sequence: 41,
                last_sequence: 42,
                events: rows.clone(),
            },
            0,
        )
        .unwrap();
        let bytes = encode_event_body(&part, &batch).unwrap();
        let decoded = decode_event_body(&part, &bytes).unwrap();
        assert_eq!(encode_event_body(&part, &decoded).unwrap(), bytes);
        assert_eq!(rows_from_event_batch(&part, &decoded).unwrap(), rows);
    }

    #[test]
    fn omitted_sequence_is_enforced_at_batch_boundary_and_then_derived() {
        let part = event_part();
        assert_eq!(part.fields.len(), 7);
        assert!(!part.fields.iter().any(|field| field.field_id > 7));
        let input = row(99, LifecycleReason::RetainedDecrement);
        let lifecycle = LifecyclePayload {
            complete: true,
            hook_epoch: 3,
            first_sequence: 99,
            last_sequence: 99,
            events: vec![input],
        };
        let batch = event_batch_from_lifecycle_payload(&part, &lifecycle, 0).unwrap();
        let recovered = rows_from_event_batch(&part, &batch).unwrap();
        assert_eq!(recovered[0].sequence, 99);
        assert_eq!(recovered[0].due_after, Some(9));

        let mut contradictory = lifecycle;
        contradictory.events[0].sequence = 98;
        assert_eq!(
            event_batch_from_lifecycle_payload(&part, &contradictory, 0),
            Err("d64 lifecycle row 0 sequence 98 != expected 99".into())
        );
        let mut bad_bounds = contradictory.clone();
        bad_bounds.events[0].sequence = 99;
        bad_bounds.last_sequence = 100;
        assert_eq!(
            event_batch_from_lifecycle_payload(&part, &bad_bounds, 0),
            Err("d64 lifecycle batch sequence bounds do not match event count".into())
        );
        let empty_with_bounds = LifecyclePayload {
            complete: true,
            hook_epoch: 3,
            first_sequence: 1,
            last_sequence: 1,
            events: Vec::new(),
        };
        assert_eq!(
            event_batch_from_lifecycle_payload(&part, &empty_with_bounds, 0),
            Err("d64 empty lifecycle batch has non-zero sequence bounds".into())
        );
        let zero_identity = LifecyclePayload {
            complete: true,
            hook_epoch: 3,
            first_sequence: 0,
            last_sequence: 0,
            events: vec![row(0, LifecycleReason::Executed)],
        };
        assert_eq!(
            event_batch_from_lifecycle_payload(&part, &zero_identity, 0),
            Err("d64 lifecycle batch cannot derive positive identities".into())
        );
    }

    #[test]
    fn only_complete_nonempty_canonical_lifecycle_payloads_become_event_batches() {
        let part = event_part();
        let incomplete = LifecyclePayload {
            complete: false,
            hook_epoch: 3,
            first_sequence: 1,
            last_sequence: 1,
            events: vec![row(1, LifecycleReason::Executed)],
        };
        assert_eq!(
            event_batch_from_lifecycle_payload(&part, &incomplete, 0),
            Err("d64 incomplete lifecycle observation requires an event-part gap".into())
        );

        let empty = LifecyclePayload {
            complete: true,
            hook_epoch: 3,
            first_sequence: 0,
            last_sequence: 0,
            events: Vec::new(),
        };
        assert_eq!(
            event_batch_from_lifecycle_payload(&part, &empty, 0),
            Err("d64 empty lifecycle observation has no event batch".into())
        );

        let mut reduced = part.clone();
        reduced
            .fields
            .retain(|field| field.field_id != field::SCOPE_RANDOM);
        let complete = LifecyclePayload {
            complete: true,
            ..incomplete
        };
        assert_eq!(
            event_batch_from_lifecycle_payload(&reduced, &complete, 0),
            Err("not the canonical d64 lifecycle event part".into())
        );
        assert_eq!(
            record_from_lifecycle_row(&reduced, &complete.events[0], 0),
            Err("not the canonical d64 lifecycle event part".into())
        );
    }

    #[test]
    fn replay_rejects_noncanonical_frontier_and_domain_impossible_values() {
        let part = event_part();
        let valid = LifecyclePayload {
            complete: true,
            hook_epoch: 3,
            first_sequence: 1,
            last_sequence: 1,
            events: vec![row(1, LifecycleReason::Executed)],
        };
        let mut batch = event_batch_from_lifecycle_payload(&part, &valid, 0).unwrap();
        batch.frontier[0] = 0;
        assert_eq!(
            rows_from_event_batch(&part, &batch),
            Err("d64 decoded lifecycle frontier has zero hook_epoch".into())
        );

        let mut batch = event_batch_from_lifecycle_payload(&part, &valid, 0).unwrap();
        batch.records[0].values[2] = Value::ContentId(0);
        assert_eq!(
            rows_from_event_batch(&part, &batch),
            Err("d64 lifecycle mapper refuses zero owner_tag_db_id".into())
        );

        let mut batch = event_batch_from_lifecycle_payload(&part, &valid, 0).unwrap();
        batch.records[0].values[3] = Value::ContentId(99);
        assert_eq!(
            rows_from_event_batch(&part, &batch),
            Err("d64 lifecycle mapper refuses out-of-range event_id".into())
        );
    }

    #[test]
    fn mapper_refuses_every_contradictory_reason_due_after_pair() {
        let part = event_part();
        for reason in [
            LifecycleReason::RetainedDecrement,
            LifecycleReason::Executed,
            LifecycleReason::FailedTriggerConsumed,
            LifecycleReason::DefinitionCancelled,
        ] {
            for due_after in [None, Some(0), Some(3), Some(9), Some(10), Some(u32::MAX)] {
                let mut candidate = row(1, reason);
                candidate.due_before = 10;
                candidate.due_after = due_after;
                let expected = match reason {
                    LifecycleReason::RetainedDecrement => Some(9),
                    _ => None,
                };
                let result = record_from_lifecycle_row(&part, &candidate, 0);
                if due_after == expected {
                    assert!(result.is_ok(), "{reason:?}, {due_after:?}");
                } else {
                    assert_eq!(
                        result,
                        Err(format!(
                            "d64 lifecycle mapper refuses {reason:?} with due_before 10 and \
                             due_after {due_after:?}; expected {expected:?}"
                        )),
                        "{reason:?}, {due_after:?}"
                    );
                }
            }
        }
        let mut zero_due = row(1, LifecycleReason::RetainedDecrement);
        zero_due.due_before = 0;
        zero_due.due_after = None;
        assert_eq!(
            record_from_lifecycle_row(&part, &zero_due, 0),
            Err("d64 retained event has zero due_before".into())
        );
    }

    #[test]
    fn no_quantization_or_derived_field_test_applies() {
        // There is no Continuous field and all inputs are exact-width integers,
        // so recipe step 8's "quantization saves bytes" test is inapplicable.
        // Sequence and due_after are event-level derived facts, not derived-field contract
        // clock-linear stored fields.
        let part = event_part();
        assert!(part
            .fields
            .iter()
            .all(|field| field.kind != Kind::Continuous));
        assert!(part.fields.iter().all(|field| field.derived.is_none()));
    }

    #[test]
    fn known_header_bytes_cover_every_offset() {
        let rows = [row(0x1112_1314_1516_1718, LifecycleReason::Executed)];
        let mut bytes = trailer(true, 0x2122_2324_2526_2728, rows[0].sequence, &rows);
        // Pin both reserved offsets independently.
        assert_eq!(&bytes[0..4], b"DEL2");
        assert_eq!(u16_at(&bytes, 4), 1);
        assert_eq!(u16_at(&bytes, 6), 48);
        assert_eq!(u16_at(&bytes, 8), 48);
        assert_eq!(u16_at(&bytes, 10), 0);
        assert_eq!(u32_at(&bytes, 12), 1);
        assert_eq!(u32_at(&bytes, 16), 1);
        assert_eq!(u64_at(&bytes, 20), 0x2122_2324_2526_2728);
        assert_eq!(u64_at(&bytes, 28), 0x1112_1314_1516_1718);
        assert_eq!(u64_at(&bytes, 36), 0x1112_1314_1516_1718);
        assert_eq!(u32_at(&bytes, 44), 0);
        assert_eq!(parse_trailer(&bytes).unwrap().events, rows);
        bytes[10] = 1;
        assert_eq!(
            parse_trailer(&bytes),
            Err("delayed_event_lifecycle header rejected".into()),
            "reserved +10 is checked"
        );
    }

    #[test]
    fn known_row_bytes_cover_every_offset() {
        let expected = LifecycleRow {
            sequence: 0x0102_0304_0506_0708,
            game_hour_raw: -123_456,
            reason: LifecycleReason::RetainedDecrement,
            owner_tag_db_id: 0x1112_1314,
            event_id: 100_099_999,
            originator_tag_db_id: 0x2122_2324,
            scope_random: 0x3132_3334_3536_3738,
            due_before: 500_000,
            due_after: Some(499_999),
        };
        let bytes = trailer(true, 7, expected.sequence, std::slice::from_ref(&expected));
        let base = TRAILER_HEADER_LEN;
        assert_eq!(u64_at(&bytes, base), expected.sequence);
        assert_eq!(i32_at(&bytes, base + 8), expected.game_hour_raw);
        assert_eq!(u32_at(&bytes, base + 12), 1);
        assert_eq!(u32_at(&bytes, base + 16), expected.owner_tag_db_id);
        assert_eq!(u32_at(&bytes, base + 20), expected.event_id);
        assert_eq!(u32_at(&bytes, base + 24), expected.originator_tag_db_id);
        assert_eq!(u32_at(&bytes, base + 28), 1);
        assert_eq!(u64_at(&bytes, base + 32), expected.scope_random);
        assert_eq!(u32_at(&bytes, base + 40), expected.due_before);
        assert_eq!(u32_at(&bytes, base + 44), expected.due_after.unwrap());
        assert_eq!(parse_trailer(&bytes).unwrap().events, vec![expected]);
    }

    #[test]
    fn split_payload_covers_every_base_header_word_and_call_order() {
        let lifecycle = trailer(true, 9, 1, &[row(1, LifecycleReason::Executed)]);
        let bytes = payload(0x100, Some(&lifecycle));
        let parts = split_payload(&bytes).unwrap();
        assert_eq!(parts.header.elapsed_wall_ms, 123);
        assert_eq!(parts.header.row_count, 0);
        assert_eq!(parts.header.observed_rows, 0);
        assert_eq!(parts.header.status_flags, 0x100);
        assert!(parts.state_rows.is_empty());
        assert_eq!(parts.trailer, Some(lifecycle.as_slice()));
        for len in 0..BASE_HEADER_LEN {
            assert_eq!(
                split_payload(&bytes[..len]).unwrap_err(),
                format!(
                    "delayed_events header is {len} byte(s), expected at least {BASE_HEADER_LEN}"
                ),
                "base header len {len}"
            );
        }
        let mut mismatched = bytes.clone();
        mismatched[12..16].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            split_payload(&mismatched).unwrap_err(),
            "delayed_events observed/packed row count mismatch"
        );
    }

    #[test]
    fn parser_rejects_every_structural_header_rule() {
        let good = trailer(true, 1, 1, &[row(1, LifecycleReason::Executed)]);
        let mut cases = Vec::new();
        let mutate = |offset: usize, replacement: &[u8]| {
            let mut bytes = good.clone();
            bytes[offset..offset + replacement.len()].copy_from_slice(replacement);
            bytes
        };
        cases.push(mutate(0, b"BAD!"));
        cases.push(mutate(4, &2u16.to_le_bytes()));
        cases.push(mutate(6, &47u16.to_le_bytes()));
        cases.push(mutate(8, &47u16.to_le_bytes()));
        cases.push(mutate(10, &1u16.to_le_bytes()));
        cases.push(mutate(12, &2u32.to_le_bytes()));
        cases.push(mutate(16, &2u32.to_le_bytes()));
        cases.push(mutate(20, &0u64.to_le_bytes()));
        cases.push(mutate(28, &2u64.to_le_bytes()));
        cases.push(mutate(36, &2u64.to_le_bytes()));
        cases.push(mutate(44, &1u32.to_le_bytes()));
        let expected = [
            "delayed_event_lifecycle header rejected",
            "delayed_event_lifecycle header rejected",
            "delayed_event_lifecycle header rejected",
            "delayed_event_lifecycle header rejected",
            "delayed_event_lifecycle header rejected",
            "delayed_event_lifecycle unknown header flags",
            "delayed_event_lifecycle count/sequence header rejected",
            "delayed_event_lifecycle count/sequence header rejected",
            "delayed_event_lifecycle count/sequence header rejected",
            "delayed_event_lifecycle count/sequence header rejected",
            "delayed_event_lifecycle header rejected",
        ];
        for (index, (bytes, expected)) in cases.iter().zip(expected).enumerate() {
            assert_eq!(
                parse_trailer(bytes),
                Err(expected.into()),
                "header case {index}"
            );
        }
        for len in 0..good.len() {
            let error = parse_trailer(&good[..len]).unwrap_err();
            if len < TRAILER_HEADER_LEN {
                assert_eq!(
                    error,
                    format!(
                        "delayed_event_lifecycle header is {len} byte(s), expected at least \
                         {TRAILER_HEADER_LEN}"
                    ),
                    "truncation {len}"
                );
            } else {
                assert_eq!(
                    error, "delayed_event_lifecycle count/sequence header rejected",
                    "truncation {len}"
                );
            }
        }
        let empty = trailer(true, 1, 0, &[]);
        assert!(parse_trailer(&empty).is_ok());
        for offset in [28usize, 36] {
            let mut bad = empty.clone();
            bad[offset..offset + 8].copy_from_slice(&1u64.to_le_bytes());
            assert_eq!(
                parse_trailer(&bad),
                Err("delayed_event_lifecycle count/sequence header rejected".into()),
                "empty sequence bound at +{offset}"
            );
        }
    }

    #[test]
    fn parser_rejects_every_row_rule_and_accepts_all_four_reasons() {
        let good = trailer(true, 1, 1, &[row(1, LifecycleReason::Executed)]);
        let base = TRAILER_HEADER_LEN;
        let mutation = |offset: usize, replacement: &[u8]| {
            let mut bytes = good.clone();
            bytes[offset..offset + replacement.len()].copy_from_slice(replacement);
            bytes
        };
        let cases = vec![
            mutation(base, &2u64.to_le_bytes()),
            mutation(base + 8, &i32::MIN.to_le_bytes()),
            mutation(base + 12, &0u32.to_le_bytes()),
            mutation(base + 16, &0u32.to_le_bytes()),
            mutation(base + 20, &99_999u32.to_le_bytes()),
            mutation(base + 20, &100_100_000u32.to_le_bytes()),
            mutation(base + 24, &0u32.to_le_bytes()),
            mutation(base + 28, &2u32.to_le_bytes()),
            mutation(base + 40, &(MAX_DUE_REMAINING_HOURS + 1).to_le_bytes()),
            mutation(base + 44, &1u32.to_le_bytes()),
        ];
        let expected = [
            "delayed_event_lifecycle row 0 rejected",
            "delayed_event_lifecycle row 0 rejected",
            "delayed_event_lifecycle row 0 reason/due transform rejected",
            "delayed_event_lifecycle row 0 rejected",
            "delayed_event_lifecycle row 0 rejected",
            "delayed_event_lifecycle row 0 rejected",
            "delayed_event_lifecycle row 0 rejected",
            "delayed_event_lifecycle row 0 rejected",
            "delayed_event_lifecycle row 0 rejected",
            "delayed_event_lifecycle row 0 reason/due transform rejected",
        ];
        for (index, (bytes, expected)) in cases.iter().zip(expected).enumerate() {
            assert_eq!(
                parse_trailer(bytes),
                Err(expected.into()),
                "row case {index}"
            );
        }
        for reason in [
            LifecycleReason::RetainedDecrement,
            LifecycleReason::Executed,
            LifecycleReason::FailedTriggerConsumed,
            LifecycleReason::DefinitionCancelled,
        ] {
            let event = row(1, reason);
            let parsed = parse_trailer(&trailer(true, 1, 1, &[event.clone()])).unwrap();
            assert_eq!(parsed.events, vec![event]);
        }
    }

    #[test]
    fn fold_settles_trailer_despite_table_failure_and_advances_continuity() {
        let mut state = State::new();
        let first = trailer(true, 7, 1, &[row(1, LifecycleReason::Executed)]);
        let outcome = fold_payload(&mut state, &payload(0x100, Some(&first))).unwrap();
        assert_eq!(outcome.table, TablePart::StoppedUnavailable);
        assert!(matches!(outcome.ledger, LedgerPart::Published(_)));
        assert_eq!(state.last_sequence(), Some(1));

        let second = trailer(true, 7, 2, &[row(2, LifecycleReason::Executed)]);
        let outcome = fold_payload(&mut state, &payload(0x100, Some(&second))).unwrap();
        assert!(matches!(outcome.ledger, LedgerPart::Published(_)));
        assert_eq!(state.last_sequence(), Some(2));
    }

    #[test]
    fn fold_carries_clean_table_refusal_beside_the_published_ledger() {
        let event = trailer(true, 7, 1, &[row(1, LifecycleReason::Executed)]);
        let mut bytes = payload(0, None);
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[
            1, 0, 0, 0, // owner
            1, 0, 0, 0, // event id
            2, 0, 0, 0, // originator-present flag
            0, 0, 0, 0, // zero originator: fold-only table error
            0, 0, 0, 0, // absent due
        ]);
        bytes.extend_from_slice(&event);

        assert!(
            check_authority_payload(&bytes).is_ok(),
            "the stream authority admits the input before the production fold"
        );
        let mut state = State::new();
        let outcome = fold_payload(&mut state, &bytes).unwrap();
        assert_eq!(
            outcome.table,
            TablePart::Refused {
                error: "delayed_events row 0 has zero originator".into()
            }
        );
        assert_eq!(
            outcome.ledger,
            LedgerPart::Published(parse_trailer(&event).unwrap())
        );
        assert_eq!(
            state.last_sequence(),
            Some(1),
            "the independently valid trailer settles despite the table refusal"
        );
    }

    #[test]
    fn incomplete_epoch_change_and_sequence_gap_stickily_poison_production_state() {
        for bad in [
            trailer(false, 1, 1, &[row(1, LifecycleReason::Executed)]),
            trailer(true, 2, 1, &[row(1, LifecycleReason::Executed)]),
        ] {
            let mut state = State::new();
            if u64_at(&bad, 20) == 2 {
                let seed = trailer(true, 1, 1, &[row(1, LifecycleReason::Executed)]);
                fold_payload(&mut state, &payload(0x100, Some(&seed))).unwrap();
            }
            let outcome = fold_payload(&mut state, &payload(0x100, Some(&bad))).unwrap();
            assert_eq!(outcome.ledger, LedgerPart::Unknown);
            assert!(state.history_failed());
            let later = trailer(true, 1, 2, &[row(2, LifecycleReason::Executed)]);
            assert_eq!(
                fold_payload(&mut state, &payload(0x100, Some(&later)))
                    .unwrap()
                    .ledger,
                LedgerPart::Unknown,
                "production poison is sticky across recovery"
            );
        }

        let mut state = State::new();
        let seed = trailer(true, 1, 1, &[row(1, LifecycleReason::Executed)]);
        fold_payload(&mut state, &payload(0x100, Some(&seed))).unwrap();
        let gap = trailer(true, 1, 3, &[row(3, LifecycleReason::Executed)]);
        assert_eq!(
            fold_payload(&mut state, &payload(0x100, Some(&gap)))
                .unwrap()
                .ledger,
            LedgerPart::Unknown
        );
    }

    #[test]
    fn missing_trailer_is_a_hole_only_after_seen_and_on_clean_table() {
        let seed = trailer(true, 1, 1, &[row(1, LifecycleReason::Executed)]);
        let mut state = State::new();
        fold_payload(&mut state, &payload(0x100, Some(&seed))).unwrap();
        assert_eq!(
            fold_payload(&mut state, &payload(0x100, None))
                .unwrap()
                .ledger,
            LedgerPart::NoObservation
        );
        assert!(!state.history_failed());
        assert_eq!(
            fold_payload(&mut state, &payload(0, None)).unwrap().ledger,
            LedgerPart::Unknown
        );
        assert!(state.history_failed());
    }

    #[test]
    fn malformed_trailer_poison_happens_before_table_status_gate() {
        let mut state = State::new();
        let error = fold_payload(&mut state, &payload(0x100, Some(b"bad")))
            .expect_err("malformed trailer refuses even on table failure");
        assert_eq!(
            error,
            "delayed_event_lifecycle header is 3 byte(s), expected at least 48"
        );
        assert!(state.history_failed());
    }

    #[test]
    fn authority_and_fold_have_their_deliberate_changed_rows_split() {
        let mut bytes = payload(0x100, None);
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&[
            1, 0, 0, 0, // owner
            1, 0, 0, 0, // event id
            0, 0, 0, 0, // flags
            0, 0, 0, 0, // originator
            0, 0, 0, 0, // due
        ]);
        assert!(
            check_authority_payload(&bytes).is_ok(),
            "authority permits <="
        );
        assert_eq!(
            split_payload(&bytes).unwrap_err(),
            "delayed_events observed/packed row count mismatch",
            "fold requires equality"
        );
    }

    #[test]
    fn arbitrary_short_inputs_never_panic_at_public_boundaries() {
        let part = event_part();
        for len in 0..256usize {
            let bytes = vec![0xff; len];
            let _ = split_payload(&bytes);
            let _ = parse_trailer(&bytes);
            let _ = check_authority_payload(&bytes);
        }
        let fake = LifecycleRow {
            sequence: u64::MAX,
            game_hour_raw: i32::MAX,
            reason: LifecycleReason::DefinitionCancelled,
            owner_tag_db_id: u32::MAX,
            event_id: u32::MAX,
            originator_tag_db_id: u32::MAX,
            scope_random: u64::MAX,
            due_before: u32::MAX,
            due_after: None,
        };
        let _ = record_from_lifecycle_row(&part, &fake, u64::MAX);
        let _typed: Result<_, crate::WireError> = crate::decode_event_body(&part, &[0xff; 32]);
    }
}
