//! A34 complete heterogeneous country-decision panel carrier (compact d87).
//!
//! DPN1 v2 preserves the exact materialized grid, not merely decision cards.
//! Runtime order, geometry, category/event structure, scroll state, text, and
//! alert membership come from the receipt-bound country-view update. The
//! selected-Present receipt proves that materialization completed before the
//! frame which the replay actually displays.

#[cfg(windows)]
pub mod a729_hook;

use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};

pub const DOMAIN_ID: u16 = 87;
pub const DOMAIN_NAME: &str = "decision_panel_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"DPN1");
pub const SCHEMA_VERSION: u32 = 2;
pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const EXACT20_FUNCTION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"712233abfbd1352f0c84ede6267a99f4de8f5406cabfd20d76046a094b5fec0a");
pub const EXACT20_DATA_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"ccca7fbfe0378821f22ed82d1a371a58ea12c24a9601fd3dd36d97bef093f0ea");
/// Exact-a729 CCountryDecisionView update/controller/country-owner join.
/// Ghidra job:
/// `20260720T130318Z-wave5-a34-country-view-owner-tag-join-ro-f6d9b428`.
pub const COUNTRY_VIEW_OWNER_TAG_MANIFEST_SHA256: [u8; 32] =
    hex32(b"a31b9fa4d886d509ceceef46819b6048331ef65065c2f07a8cc086e0868dd07a");
pub const PRODUCTION_SCHEDULING_ENABLED: bool = false;
pub const LIVE_FIXTURE_ACCEPTED: bool = false;
pub const MAX_ROWS: usize = 32_768;
pub const MAX_TEXT_BYTES: usize = 16_384;

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut output = [0; 32];
    let mut index = 0;
    while index < 32 {
        output[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    output
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityState {
    Unavailable,
    Exact20Receipt,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Available,
    Unavailable,
    Active,
    OnCooldown,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Pending,
    Running,
    Completed,
    Failed,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelChildKind {
    Category,
    FixedHeader,
    Event,
    Decision,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Country(String),
    State(u32),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cost {
    pub political_power: Option<i64>,
    pub custom_cost_available: Option<bool>,
    pub formatted_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPresentStamp {
    pub frame_hour: u64,
    pub selected_present_ordinal: u32,
    pub selected_present_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub selected_present_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionGridState {
    pub clip_left: i32,
    pub clip_top: i32,
    pub clip_right: i32,
    pub clip_bottom: i32,
    pub content_origin_x: i32,
    pub content_origin_y: i32,
    pub content_extent_width: i32,
    pub content_extent_height: i32,
    pub scroll_min: i64,
    pub scroll_max: i64,
    pub scroll_target: i64,
    pub scroll_current: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionPanelChild {
    pub presentation_index: u32,
    pub kind: PanelChildKind,
    pub decision_row_index: Option<u32>,
    pub identity: String,
    pub name_text: Option<String>,
    pub secondary_text: Option<String>,
    pub expanded: Option<bool>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionRow {
    pub decision_id: String,
    pub category_id: String,
    /// Ordinal in `DecisionPanelSnapshot.children`, not in the decision-only
    /// subset.
    pub presentation_index: u32,
    pub availability: Availability,
    pub lifecycle: Lifecycle,
    pub target: Option<Target>,
    pub cost: Option<Cost>,
    pub remaining_timer_days: Option<i32>,
    pub cooldown_remaining_days: Option<i32>,
    pub name_text: String,
    pub timer_text: Option<String>,
    pub alert_to_take: bool,
    pub alert_timeout: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionPanelSnapshot {
    pub source_sequence: u64,
    pub producer_complete: bool,
    pub country_tag: String,
    pub decision_catalog_sha256: String,
    pub materialization_qpc_ticks: u64,
    pub selected_present: SelectedPresentStamp,
    pub grid: DecisionGridState,
    pub children: Vec<DecisionPanelChild>,
    pub available_alert_count: u32,
    pub timeout_alert_count: u32,
    pub rows: Vec<DecisionRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionPanelFrame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub authority: AuthorityState,
    pub snapshot: Option<DecisionPanelSnapshot>,
}

pub trait ExactDecisionPanelSource {
    fn is_exact_a729_materializer_source(&self) -> bool;
    fn read_decision_panel(&self, frame_hour: u64) -> Result<DecisionPanelSnapshot, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    ActivationDisabled,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}

fn text_ok(value: &str, required: bool) -> bool {
    value.len() <= MAX_TEXT_BYTES
        && (!required || !value.is_empty())
        && !value.as_bytes().contains(&0)
}

fn sha256_ok(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn event_identity_ok(value: &str) -> bool {
    value
        .strip_prefix("event_")
        .is_some_and(|id| {
            !id.is_empty()
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && id.parse::<u32>().is_ok()
        })
}

pub fn validate_snapshot(snapshot: &DecisionPanelSnapshot) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0
        || !snapshot.producer_complete
        || !text_ok(&snapshot.country_tag, true)
        || !sha256_ok(&snapshot.decision_catalog_sha256)
        || snapshot.materialization_qpc_ticks == 0
        || snapshot.selected_present.selected_present_qpc_ticks == 0
        || snapshot.selected_present.qpc_frequency_hz == 0
        || !sha256_ok(
            &snapshot
                .selected_present
                .selected_present_receipt_sha256,
        )
        || snapshot.materialization_qpc_ticks
            > snapshot.selected_present.selected_present_qpc_ticks
        || snapshot.rows.len() > MAX_ROWS
        || snapshot.children.len() > MAX_ROWS
    {
        return invalid("DPN1 snapshot/selected-Present authority incomplete");
    }
    let grid = &snapshot.grid;
    if grid.clip_right <= grid.clip_left
        || grid.clip_bottom <= grid.clip_top
        || grid.content_extent_width < 0
        || grid.content_extent_height < 0
        || grid.scroll_min > grid.scroll_max
        || !(grid.scroll_min..=grid.scroll_max).contains(&grid.scroll_target)
        || !(grid.scroll_min..=grid.scroll_max).contains(&grid.scroll_current)
    {
        return invalid("DPN1 grid clip/content/scroll state invalid");
    }

    let mut referenced_rows = BTreeSet::new();
    for (ordinal, child) in snapshot.children.iter().enumerate() {
        if child.presentation_index as usize != ordinal
            || !text_ok(&child.identity, true)
            || child.name_text.as_deref().is_some_and(|v| !text_ok(v, true))
            || child
                .secondary_text
                .as_deref()
                .is_some_and(|v| !text_ok(v, true))
            || child.width <= 0
            || child.height <= 0
        {
            return invalid("DPN1 heterogeneous child order/content invalid");
        }
        match (child.kind, child.decision_row_index) {
            (PanelChildKind::Decision, Some(row_index)) => {
                let Some(row) = snapshot.rows.get(row_index as usize) else {
                    return invalid("DPN1 decision child row index out of range");
                };
                if !referenced_rows.insert(row_index)
                    || row.presentation_index != child.presentation_index
                    || row.decision_id != child.identity
                    || child.name_text.as_deref() != Some(row.name_text.as_str())
                    || child.secondary_text.as_deref() != row.timer_text.as_deref()
                {
                    return invalid("DPN1 decision child/row join invalid");
                }
            }
            (PanelChildKind::Decision, None) | (_, Some(_)) => {
                return invalid("DPN1 structural child decision reference invalid")
            }
            _ => {}
        }
        match child.kind {
            PanelChildKind::Category
                if child.name_text.is_none() || child.expanded.is_none() =>
            {
                return invalid("DPN1 category child lacks exact name/expansion state")
            }
            PanelChildKind::FixedHeader
                if child.identity != "event_header"
                    || child.secondary_text.is_some()
                    || child.expanded.is_some() =>
            {
                return invalid("DPN1 fixed-header structural state invalid")
            }
            PanelChildKind::Event
                if !event_identity_ok(&child.identity)
                    || child.name_text.is_none()
                    || child.expanded.is_some() =>
            {
                return invalid("DPN1 event structural state invalid")
            }
            PanelChildKind::Decision if child.expanded.is_some() => {
                return invalid("DPN1 decision child has category expansion state")
            }
            _ => {}
        }
    }
    if referenced_rows.len() != snapshot.rows.len() {
        return invalid("DPN1 decision rows are not bijective with grid children");
    }

    let mut identities = BTreeSet::new();
    let mut take = 0u32;
    let mut timeout = 0u32;
    for row in &snapshot.rows {
        if row.presentation_index as usize >= snapshot.children.len()
            || !text_ok(&row.decision_id, true)
            || !text_ok(&row.category_id, true)
            || !text_ok(&row.name_text, true)
            || row.timer_text.as_deref().is_some_and(|v| !text_ok(v, true))
            || row.remaining_timer_days.is_some_and(|v| v < 0)
            || row.cooldown_remaining_days.is_some_and(|v| v < 0)
        {
            return invalid("DPN1 decision row identity/text/timer invalid");
        }
        if !identities.insert((row.decision_id.clone(), row.target.clone())) {
            return invalid("DPN1 duplicate decision target");
        }
        match &row.target {
            Some(Target::Country(value)) if text_ok(value, true) => {}
            Some(Target::State(value)) if *value > 0 => {}
            Some(_) => return invalid("DPN1 target invalid"),
            None => {}
        }
        if let Some(cost) = &row.cost {
            if cost
                .formatted_text
                .as_deref()
                .is_some_and(|value| !text_ok(value, true))
            {
                return invalid("DPN1 cost text invalid");
            }
        }
        if row.remaining_timer_days.is_some() || row.cooldown_remaining_days.is_some() {
            if row.timer_text.is_none() {
                return invalid("DPN1 timer value lacks exact text");
            }
        } else if row.timer_text.is_some() {
            return invalid("DPN1 timer text lacks exact value");
        }
        if row.alert_to_take {
            take = take
                .checked_add(1)
                .ok_or_else(|| CarrierError::Invalid("DPN1 alert count overflow".into()))?;
        }
        if row.alert_timeout {
            timeout = timeout
                .checked_add(1)
                .ok_or_else(|| CarrierError::Invalid("DPN1 alert count overflow".into()))?;
        }
    }
    if take != snapshot.available_alert_count || timeout != snapshot.timeout_alert_count {
        return invalid("DPN1 alert membership/count mismatch");
    }
    Ok(())
}

pub fn validate_frame(frame: &DecisionPanelFrame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::Unavailable if frame.status_flags != 0 && frame.snapshot.is_none() => Ok(()),
        AuthorityState::Exact20Receipt if frame.status_flags == 0 && frame.snapshot.is_some() => {
            let snapshot = frame.snapshot.as_ref().expect("checked snapshot");
            if snapshot.selected_present.frame_hour != frame.frame_hour {
                return invalid("DPN1 selected-Present hour mismatch");
            }
            validate_snapshot(snapshot)
        }
        _ => invalid("DPN1 authority/availability mismatch"),
    }
}

pub fn collect_and_encode<S: ExactDecisionPanelSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    if !PRODUCTION_SCHEDULING_ENABLED || !LIVE_FIXTURE_ACCEPTED {
        return Err(CarrierError::ActivationDisabled);
    }
    if !source.is_exact_a729_materializer_source() {
        return Err(CarrierError::Source(
            "source is not the receipt-bound materializer TLS bracket".into(),
        ));
    }
    let snapshot = source
        .read_decision_panel(frame_hour)
        .map_err(CarrierError::Source)?;
    encode_payload(&DecisionPanelFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags: 0,
        authority: AuthorityState::Exact20Receipt,
        snapshot: Some(snapshot),
    })
}

pub fn encode_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&DecisionPanelFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority: AuthorityState::Unavailable,
        snapshot: None,
    })
}

pub fn encode_payload(frame: &DecisionPanelFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut output = Vec::new();
    u32v(&mut output, MAGIC);
    u32v(&mut output, SCHEMA_VERSION);
    u32v(
        &mut output,
        match frame.authority {
            AuthorityState::Unavailable => 0,
            AuthorityState::Exact20Receipt => 1,
        },
    );
    u32v(&mut output, frame.status_flags);
    u64v(&mut output, frame.elapsed_wall_ms);
    u64v(&mut output, frame.frame_hour);
    output.extend_from_slice(&EXECUTABLE_SHA256);
    output.extend_from_slice(&EXACT20_FUNCTION_MANIFEST_SHA256);
    output.extend_from_slice(&EXACT20_DATA_ARTIFACT_SHA256);
    output.push(u8::from(frame.snapshot.is_some()));
    output.extend_from_slice(&[0; 3]);
    if let Some(snapshot) = &frame.snapshot {
        u64v(&mut output, snapshot.source_sequence);
        textv(&mut output, &snapshot.country_tag);
        textv(&mut output, &snapshot.decision_catalog_sha256);
        u64v(&mut output, snapshot.materialization_qpc_ticks);
        u64v(&mut output, snapshot.selected_present.frame_hour);
        u32v(&mut output, snapshot.selected_present.selected_present_ordinal);
        u64v(
            &mut output,
            snapshot.selected_present.selected_present_qpc_ticks,
        );
        u64v(&mut output, snapshot.selected_present.qpc_frequency_hz);
        textv(
            &mut output,
            &snapshot
                .selected_present
                .selected_present_receipt_sha256,
        );
        encode_grid(&mut output, &snapshot.grid);
        u32v(&mut output, snapshot.children.len() as u32);
        for child in &snapshot.children {
            u32v(&mut output, child.presentation_index);
            output.push(child_kind_raw(child.kind));
            let mut flags = 0u8;
            if child.name_text.is_some() {
                flags |= 1;
            }
            if child.secondary_text.is_some() {
                flags |= 2;
            }
            if child.expanded.is_some() {
                flags |= 4;
            }
            if child.expanded == Some(true) {
                flags |= 8;
            }
            if child.decision_row_index.is_some() {
                flags |= 16;
            }
            output.push(flags);
            output.extend_from_slice(&[0; 2]);
            i32v(&mut output, child.x);
            i32v(&mut output, child.y);
            i32v(&mut output, child.width);
            i32v(&mut output, child.height);
            u32v(&mut output, child.decision_row_index.unwrap_or(u32::MAX));
            textv(&mut output, &child.identity);
            opt_textv(&mut output, child.name_text.as_deref());
            opt_textv(&mut output, child.secondary_text.as_deref());
        }
        u32v(&mut output, snapshot.available_alert_count);
        u32v(&mut output, snapshot.timeout_alert_count);
        u32v(&mut output, snapshot.rows.len() as u32);
        for row in &snapshot.rows {
            encode_row(&mut output, row);
        }
    }
    Ok(output)
}

pub fn decode_payload(payload: &[u8]) -> Result<DecisionPanelFrame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("DPN1 magic/schema mismatch");
    }
    let authority = match reader.u32()? {
        0 => AuthorityState::Unavailable,
        1 => AuthorityState::Exact20Receipt,
        _ => return invalid("DPN1 authority unknown"),
    };
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    if reader.bytes(32)? != EXECUTABLE_SHA256
        || reader.bytes(32)? != EXACT20_FUNCTION_MANIFEST_SHA256
        || reader.bytes(32)? != EXACT20_DATA_ARTIFACT_SHA256
    {
        return invalid("DPN1 receipt identity mismatch");
    }
    let has_snapshot = reader.u8()?;
    if has_snapshot > 1 || reader.bytes(3)? != [0, 0, 0] {
        return invalid("DPN1 availability invalid");
    }
    let snapshot = if has_snapshot == 1 {
        let source_sequence = reader.u64()?;
        let country_tag = reader.text()?;
        let decision_catalog_sha256 = reader.text()?;
        let materialization_qpc_ticks = reader.u64()?;
        let selected_present = SelectedPresentStamp {
            frame_hour: reader.u64()?,
            selected_present_ordinal: reader.u32()?,
            selected_present_qpc_ticks: reader.u64()?,
            qpc_frequency_hz: reader.u64()?,
            selected_present_receipt_sha256: reader.text()?,
        };
        let grid = decode_grid(&mut reader)?;
        let child_count = reader.u32()? as usize;
        if child_count > MAX_ROWS {
            return invalid("DPN1 child count exceeds bound");
        }
        let mut children = Vec::with_capacity(child_count);
        for _ in 0..child_count {
            let presentation_index = reader.u32()?;
            let kind = child_kind_from(reader.u8()?)?;
            let flags = reader.u8()?;
            if flags & !0x1f != 0 || reader.bytes(2)? != [0, 0] {
                return invalid("DPN1 child flags/reserved bytes invalid");
            }
            let x = reader.i32()?;
            let y = reader.i32()?;
            let width = reader.i32()?;
            let height = reader.i32()?;
            let raw_row_index = reader.u32()?;
            let identity = reader.text()?;
            let name_text = reader.opt_text()?;
            let secondary_text = reader.opt_text()?;
            if (flags & 1 != 0) != name_text.is_some()
                || (flags & 2 != 0) != secondary_text.is_some()
            {
                return invalid("DPN1 child optional text flags mismatch");
            }
            let expanded = if flags & 4 != 0 {
                Some(flags & 8 != 0)
            } else {
                if flags & 8 != 0 {
                    return invalid("DPN1 child expanded flag invalid");
                }
                None
            };
            let decision_row_index = if flags & 16 != 0 {
                if raw_row_index == u32::MAX {
                    return invalid("DPN1 present child row index uses absent sentinel");
                }
                Some(raw_row_index)
            } else {
                if raw_row_index != u32::MAX {
                    return invalid("DPN1 absent child row index is not sentinel");
                }
                None
            };
            children.push(DecisionPanelChild {
                presentation_index,
                kind,
                decision_row_index,
                identity,
                name_text,
                secondary_text,
                expanded,
                x,
                y,
                width,
                height,
            });
        }
        let available_alert_count = reader.u32()?;
        let timeout_alert_count = reader.u32()?;
        let row_count = reader.u32()? as usize;
        if row_count > MAX_ROWS {
            return invalid("DPN1 row count exceeds bound");
        }
        let mut rows = Vec::with_capacity(row_count);
        for _ in 0..row_count {
            rows.push(decode_row(&mut reader)?);
        }
        Some(DecisionPanelSnapshot {
            source_sequence,
            producer_complete: true,
            country_tag,
            decision_catalog_sha256,
            materialization_qpc_ticks,
            selected_present,
            grid,
            children,
            available_alert_count,
            timeout_alert_count,
            rows,
        })
    } else {
        None
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = DecisionPanelFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn encode_grid(output: &mut Vec<u8>, grid: &DecisionGridState) {
    for value in [
        grid.clip_left,
        grid.clip_top,
        grid.clip_right,
        grid.clip_bottom,
        grid.content_origin_x,
        grid.content_origin_y,
        grid.content_extent_width,
        grid.content_extent_height,
    ] {
        i32v(output, value);
    }
    for value in [grid.scroll_min, grid.scroll_max, grid.scroll_target, grid.scroll_current] {
        i64v(output, value);
    }
}

fn decode_grid(reader: &mut Reader<'_>) -> Result<DecisionGridState, CarrierError> {
    Ok(DecisionGridState {
        clip_left: reader.i32()?,
        clip_top: reader.i32()?,
        clip_right: reader.i32()?,
        clip_bottom: reader.i32()?,
        content_origin_x: reader.i32()?,
        content_origin_y: reader.i32()?,
        content_extent_width: reader.i32()?,
        content_extent_height: reader.i32()?,
        scroll_min: reader.i64()?,
        scroll_max: reader.i64()?,
        scroll_target: reader.i64()?,
        scroll_current: reader.i64()?,
    })
}

fn encode_row(output: &mut Vec<u8>, row: &DecisionRow) {
    u32v(output, row.presentation_index);
    output.push(availability_raw(row.availability));
    output.push(lifecycle_raw(row.lifecycle));
    output.push(match &row.target {
        None => 0,
        Some(Target::Country(_)) => 1,
        Some(Target::State(_)) => 2,
    });
    let mut flags = 0u16;
    if row.cost.is_some() { flags |= 1; }
    if row.cost.as_ref().and_then(|v| v.political_power).is_some() { flags |= 2; }
    if row.cost.as_ref().and_then(|v| v.custom_cost_available).is_some() { flags |= 4; }
    if row.cost.as_ref().and_then(|v| v.custom_cost_available) == Some(true) { flags |= 8; }
    if row.cost.as_ref().and_then(|v| v.formatted_text.as_ref()).is_some() { flags |= 16; }
    if row.remaining_timer_days.is_some() { flags |= 32; }
    if row.cooldown_remaining_days.is_some() { flags |= 64; }
    if row.timer_text.is_some() { flags |= 128; }
    if row.alert_to_take { flags |= 256; }
    if row.alert_timeout { flags |= 512; }
    u16v(output, flags);
    i32v(output, row.remaining_timer_days.unwrap_or(-1));
    i32v(output, row.cooldown_remaining_days.unwrap_or(-1));
    i64v(output, row.cost.as_ref().and_then(|v| v.political_power).unwrap_or(0));
    u32v(output, match &row.target { Some(Target::State(v)) => *v, _ => 0 });
    textv(output, &row.decision_id);
    textv(output, &row.category_id);
    textv(output, match &row.target { Some(Target::Country(v)) => v, _ => "" });
    textv(output, &row.name_text);
    opt_textv(output, row.timer_text.as_deref());
    opt_textv(output, row.cost.as_ref().and_then(|v| v.formatted_text.as_deref()));
}

fn decode_row(reader: &mut Reader<'_>) -> Result<DecisionRow, CarrierError> {
    let presentation_index = reader.u32()?;
    let availability = availability_from(reader.u8()?)?;
    let lifecycle = lifecycle_from(reader.u8()?)?;
    let target_kind = reader.u8()?;
    let flags = reader.u16()?;
    if flags & !0x3ff != 0 {
        return invalid("DPN1 row flags unknown");
    }
    let remaining_timer_days = optional_nonnegative(reader.i32()?, flags & 32 != 0)?;
    let cooldown_remaining_days = optional_nonnegative(reader.i32()?, flags & 64 != 0)?;
    let political_power = reader.i64()?;
    let state_id = reader.u32()?;
    let decision_id = reader.text()?;
    let category_id = reader.text()?;
    let target_country = reader.text()?;
    let name_text = reader.text()?;
    let timer_text = reader.opt_text()?;
    let cost_text = reader.opt_text()?;
    let target = match target_kind {
        0 if state_id == 0 && target_country.is_empty() => None,
        1 if state_id == 0 && !target_country.is_empty() => Some(Target::Country(target_country)),
        2 if state_id > 0 && target_country.is_empty() => Some(Target::State(state_id)),
        _ => return invalid("DPN1 target encoding invalid"),
    };
    let cost = if flags & 1 != 0 {
        Some(Cost {
            political_power: (flags & 2 != 0).then_some(political_power),
            custom_cost_available: if flags & 4 != 0 { Some(flags & 8 != 0) } else { None },
            formatted_text: cost_text,
        })
    } else {
        if flags & (2 | 4 | 8 | 16) != 0 || cost_text.is_some() {
            return invalid("DPN1 absent cost has fields");
        }
        None
    };
    if (flags & 128 != 0) != timer_text.is_some()
        || (flags & 16 != 0) != cost.as_ref().and_then(|v| v.formatted_text.as_ref()).is_some()
    {
        return invalid("DPN1 optional text flags mismatch");
    }
    Ok(DecisionRow {
        decision_id,
        category_id,
        presentation_index,
        availability,
        lifecycle,
        target,
        cost,
        remaining_timer_days,
        cooldown_remaining_days,
        name_text,
        timer_text,
        alert_to_take: flags & 256 != 0,
        alert_timeout: flags & 512 != 0,
    })
}

fn child_kind_raw(value: PanelChildKind) -> u8 {
    match value {
        PanelChildKind::Category => 0,
        PanelChildKind::FixedHeader => 1,
        PanelChildKind::Event => 2,
        PanelChildKind::Decision => 3,
    }
}

fn child_kind_from(value: u8) -> Result<PanelChildKind, CarrierError> {
    match value {
        0 => Ok(PanelChildKind::Category),
        1 => Ok(PanelChildKind::FixedHeader),
        2 => Ok(PanelChildKind::Event),
        3 => Ok(PanelChildKind::Decision),
        _ => invalid("DPN1 child kind unknown"),
    }
}

fn availability_raw(value: Availability) -> u8 {
    match value { Availability::Available => 0, Availability::Unavailable => 1, Availability::Active => 2, Availability::OnCooldown => 3 }
}
fn availability_from(value: u8) -> Result<Availability, CarrierError> {
    match value { 0 => Ok(Availability::Available), 1 => Ok(Availability::Unavailable), 2 => Ok(Availability::Active), 3 => Ok(Availability::OnCooldown), _ => invalid("DPN1 availability enum unknown") }
}
fn lifecycle_raw(value: Lifecycle) -> u8 {
    match value { Lifecycle::Pending => 0, Lifecycle::Running => 1, Lifecycle::Completed => 2, Lifecycle::Failed => 3, Lifecycle::Removed => 4 }
}
fn lifecycle_from(value: u8) -> Result<Lifecycle, CarrierError> {
    match value { 0 => Ok(Lifecycle::Pending), 1 => Ok(Lifecycle::Running), 2 => Ok(Lifecycle::Completed), 3 => Ok(Lifecycle::Failed), 4 => Ok(Lifecycle::Removed), _ => invalid("DPN1 lifecycle enum unknown") }
}
fn optional_nonnegative(value: i32, present: bool) -> Result<Option<i32>, CarrierError> {
    if present && value >= 0 { Ok(Some(value)) } else if !present && value == -1 { Ok(None) } else { invalid("DPN1 optional timer encoding invalid") }
}
fn u16v(output: &mut Vec<u8>, value: u16) { output.extend_from_slice(&value.to_le_bytes()); }
fn u32v(output: &mut Vec<u8>, value: u32) { output.extend_from_slice(&value.to_le_bytes()); }
fn i32v(output: &mut Vec<u8>, value: i32) { output.extend_from_slice(&value.to_le_bytes()); }
fn u64v(output: &mut Vec<u8>, value: u64) { output.extend_from_slice(&value.to_le_bytes()); }
fn i64v(output: &mut Vec<u8>, value: i64) { output.extend_from_slice(&value.to_le_bytes()); }
fn textv(output: &mut Vec<u8>, value: &str) { u32v(output, value.len() as u32); output.extend_from_slice(value.as_bytes()); }
fn opt_textv(output: &mut Vec<u8>, value: Option<&str>) { match value { Some(value) => { output.push(1); textv(output, value); } None => output.push(0) } }

struct Reader<'a> { payload: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn bytes(&mut self, count: usize) -> Result<&'a [u8], CarrierError> { let end = self.at.checked_add(count).ok_or(CarrierError::Truncated)?; let bytes = self.payload.get(self.at..end).ok_or(CarrierError::Truncated)?; self.at = end; Ok(bytes) }
    fn u8(&mut self) -> Result<u8, CarrierError> { Ok(self.bytes(1)?[0]) }
    fn u16(&mut self) -> Result<u16, CarrierError> { Ok(u16::from_le_bytes(self.bytes(2)?.try_into().expect("length"))) }
    fn u32(&mut self) -> Result<u32, CarrierError> { Ok(u32::from_le_bytes(self.bytes(4)?.try_into().expect("length"))) }
    fn i32(&mut self) -> Result<i32, CarrierError> { Ok(i32::from_le_bytes(self.bytes(4)?.try_into().expect("length"))) }
    fn u64(&mut self) -> Result<u64, CarrierError> { Ok(u64::from_le_bytes(self.bytes(8)?.try_into().expect("length"))) }
    fn i64(&mut self) -> Result<i64, CarrierError> { Ok(i64::from_le_bytes(self.bytes(8)?.try_into().expect("length"))) }
    fn text(&mut self) -> Result<String, CarrierError> { let count = self.u32()? as usize; if count > MAX_TEXT_BYTES { return invalid("DPN1 text exceeds bound"); } String::from_utf8(self.bytes(count)?.to_vec()).map_err(|_| CarrierError::Invalid("DPN1 text invalid UTF-8".into())) }
    fn opt_text(&mut self) -> Result<Option<String>, CarrierError> { match self.u8()? { 0 => Ok(None), 1 => Ok(Some(self.text()?)), _ => invalid("DPN1 optional text marker invalid") } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> DecisionPanelFrame {
        let row = DecisionRow {
            decision_id: "decision_one".into(),
            category_id: "category_one".into(),
            presentation_index: 1,
            availability: Availability::Available,
            lifecycle: Lifecycle::Pending,
            target: None,
            cost: Some(Cost { political_power: Some(50), custom_cost_available: None, formatted_text: Some("50".into()) }),
            remaining_timer_days: None,
            cooldown_remaining_days: None,
            name_text: "Exact decision".into(),
            timer_text: None,
            alert_to_take: true,
            alert_timeout: false,
        };
        DecisionPanelFrame {
            elapsed_wall_ms: 1,
            frame_hour: 55,
            status_flags: 0,
            authority: AuthorityState::Exact20Receipt,
            snapshot: Some(DecisionPanelSnapshot {
                source_sequence: 7,
                producer_complete: true,
                country_tag: "ENG".into(),
                decision_catalog_sha256: "a".repeat(64),
                materialization_qpc_ticks: 90,
                selected_present: SelectedPresentStamp { frame_hour: 55, selected_present_ordinal: 0, selected_present_qpc_ticks: 100, qpc_frequency_hz: 10_000_000, selected_present_receipt_sha256: "b".repeat(64) },
                grid: DecisionGridState { clip_left: 0, clip_top: 0, clip_right: 502, clip_bottom: 600, content_origin_x: 0, content_origin_y: 0, content_extent_width: 502, content_extent_height: 99, scroll_min: 0, scroll_max: 0, scroll_target: 0, scroll_current: 0 },
                children: vec![
                    DecisionPanelChild { presentation_index: 0, kind: PanelChildKind::Category, decision_row_index: None, identity: "category_one".into(), name_text: Some("Category".into()), secondary_text: None, expanded: Some(true), x: 0, y: 0, width: 502, height: 58 },
                    DecisionPanelChild { presentation_index: 1, kind: PanelChildKind::Decision, decision_row_index: Some(0), identity: "decision_one".into(), name_text: Some("Exact decision".into()), secondary_text: None, expanded: None, x: 0, y: 58, width: 502, height: 41 },
                ],
                available_alert_count: 1,
                timeout_alert_count: 0,
                rows: vec![row],
            }),
        }
    }

    #[test]
    fn roundtrip_preserves_heterogeneous_order_and_selected_present() {
        let frame = frame();
        assert_eq!(decode_payload(&encode_payload(&frame).unwrap()).unwrap(), frame);
    }

    #[test]
    fn orphan_decision_row_rejects_whole_panel() {
        let mut frame = frame();
        frame.snapshot.as_mut().unwrap().children.pop();
        assert!(encode_payload(&frame).is_err());
    }

    #[test]
    fn production_source_remains_inert() {
        struct Source;
        impl ExactDecisionPanelSource for Source {
            fn is_exact_a729_materializer_source(&self) -> bool { true }
            fn read_decision_panel(&self, _: u64) -> Result<DecisionPanelSnapshot, String> { Ok(frame().snapshot.unwrap()) }
        }
        assert_eq!(collect_and_encode(&Source, 55, 0), Err(CarrierError::ActivationDisabled));
    }
}
