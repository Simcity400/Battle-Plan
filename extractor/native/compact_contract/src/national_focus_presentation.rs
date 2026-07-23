//! A31 exact national-focus presentation transport (compact domain 89).
//!
//! The wire owns only pointer-free identities. Process addresses are accepted
//! solely by the lane-local a729 observer and must be replaced with stable
//! focus-definition ids before a [`Snapshot`] reaches this module.

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 89;
pub const DOMAIN_NAME: &str = "national_focus_presentation_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"NFP1");
pub const SCHEMA_VERSION: u32 = 3;
pub const MAX_VISIBLE_ITEMS: usize = 131_072;
pub const MAX_UI_PASS_OPERATIONS: usize = 1_048_576;
pub const MAX_TEXT_BYTES: usize = 65_536;
pub const AUTHORED_SURFACE_IDENTITY: &str = "national_focus_canvas";
pub const RUNTIME_ROOT_IDENTITY: &str = "nationalfocusview";
pub const EFFECTIVE_INTERFACE_WINNER: &str = "interface/nationalfocusview.gui";
pub const EXECUTABLE_SHA256: [u8; 32] = hex32(
    b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
);
pub const EXACT20_FUNCTION_MANIFEST_SHA256: [u8; 32] = hex32(
    b"b319dd1b721ed6a13e47088b7def034111f9882631080bffbf049fbb79265a2f",
);
pub const EXACT20_JOB_RECEIPT_SHA256: [u8; 32] = hex32(
    b"b1c9d8c0678956f17bea2e7f9e06e7ff4baf31254c11b3c983a9a5ca9ae2d04b",
);

/// The ordinary collector owns the exact passive hook and selected-Present
/// transaction. Missing callbacks emit typed unavailable; fixtures are not a
/// global production gate.
pub const PRODUCTION_SCHEDULING_ENABLED: bool = true;
pub const LIVE_FIXTURE_ACCEPTED: bool = false;

const ITEM_FLAG_HIGHLIGHT: u16 = 1;
const ITEM_FLAG_HISTORICAL: u16 = 1 << 1;
const ITEM_FLAG_JOINT_COUNTRY: u16 = 1 << 2;
const ITEM_KNOWN_FLAGS: u16 =
    ITEM_FLAG_HIGHLIGHT | ITEM_FLAG_HISTORICAL | ITEM_FLAG_JOINT_COUNTRY;

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
    while index < output.len() {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusPresentationItem {
    pub focus_id: String,
    pub source_ordinal: u32,
    /// Final item+0x540 state: 0 unavailable, 1 complete, 2 available, 3 active.
    pub presentation_state_code: u8,
    /// Exact final integer passed by 0x141bb5df0, never a progress-days estimate.
    pub display_progress_percent: u8,
    pub highlight_glow_visible: bool,
    pub historical_marker_visible: bool,
    pub joint_viewing_country_tag: Option<String>,
    /// Exact selected-Present operations emitted for this focus. These are
    /// measured GUI-pass ordinals, not inferred widget or asset counts.
    pub selected_present_ui_operation_ordinals: Vec<u32>,
    /// Exact screen-space union of those operations in the selected Present.
    pub selected_present_screen_rect: RectI32,
}

impl FocusPresentationItem {
    pub fn available(&self) -> bool {
        self.presentation_state_code == 2
    }

    pub fn continuous_glow_visible(&self) -> bool {
        self.presentation_state_code == 3
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub source_sequence: u64,
    pub producer_complete: bool,
    pub country_tag: String,
    pub view_id: String,
    pub producer_record_identity: String,
    pub expected_visible_item_count: u32,
    pub visible_items: Vec<FocusPresentationItem>,
    /// Exact selected-Present operations owned by the registered focus-canvas
    /// root but not by one focus item (backgrounds, connectors, viewport).
    pub selected_present_canvas_operation_ordinals: Vec<u32>,
    /// Strictly ordered complete union of every focus-item and canvas-level
    /// operation owned by the registered runtime root.
    pub selected_present_ui_operation_order: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectI32 {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl RectI32 {
    pub const EMPTY: Self = Self { x: 0, y: 0, width: 0, height: 0 };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPresentBinding {
    /// QPC frontier after the paired dynamic/detail refresh completed.
    pub materialization_qpc_ticks: u64,
    pub selected_present_global_order: u32,
    pub selected_present_candidate_id: u64,
    pub selected_present_ordinal: u32,
    pub selected_present_qpc_ticks: u64,
    pub selected_present_cutoff_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    /// Canonical portable-render-stream `SelectedPresentReceipt` digest.
    pub selected_present_receipt_sha256: String,
    /// Canonical `AuthenticatedUiPass::pass_identity_sha256`.
    pub selected_present_pass_identity_sha256: String,
    /// Pointer-free owner token shared by every carried focus operation.
    pub selected_present_engine_instance_token: u64,
    pub authored_surface_identity: String,
    pub runtime_root_identity: String,
    pub effective_interface_winner: String,
    pub render_target_extent_pixels: [i32; 2],
    pub client_extent_pixels: [i32; 2],
    pub clip_rect: RectI32,
    pub content_extent_pixels: [i32; 2],
    pub viewport_extent_pixels: [i32; 2],
    /// Exact fixed-5 screen-space origin and explicit pan translation.
    pub canvas_origin_pixels_fixed5: [i64; 2],
    pub pan_pixels_fixed5: [i64; 2],
    /// Raw IEEE-754 X/Y scale bits from the selected Present.
    pub canvas_scale_bits: [u32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub authority: AuthorityState,
    pub selected_present: Option<SelectedPresentBinding>,
    pub snapshot: Option<Snapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    ActivationDisabled,
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub trait ExactNationalFocusPresentationSource {
    fn is_exact_a729_focus_view_source(&self) -> bool;
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn collect_snapshot(&self, frame_hour: u64, country_tag: &str) -> Result<Snapshot, String>;
}

pub fn collect_and_validate<S: ExactNationalFocusPresentationSource>(
    source: &S,
    frame_hour: u64,
    country_tag: &str,
) -> Result<Snapshot, CarrierError> {
    if !source.is_exact_a729_focus_view_source()
        || source.producer_manifest_sha256() != EXACT20_FUNCTION_MANIFEST_SHA256
    {
        return Err(CarrierError::UnauthenticatedSource);
    }
    let snapshot = source
        .collect_snapshot(frame_hour, country_tag)
        .map_err(CarrierError::Source)?;
    if snapshot.country_tag != country_tag {
        return invalid("requested and returned country identities differ");
    }
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn collect_and_encode<S: ExactNationalFocusPresentationSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    country_tag: &str,
    selected_present: SelectedPresentBinding,
) -> Result<Vec<u8>, CarrierError> {
    if !PRODUCTION_SCHEDULING_ENABLED {
        return Err(CarrierError::ActivationDisabled);
    }
    let snapshot = collect_and_validate(source, frame_hour, country_tag)?;
    encode_payload(&Frame {
        elapsed_wall_ms,
        frame_hour,
        status_flags: 0,
        authority: AuthorityState::Exact20Receipt,
        selected_present: Some(selected_present),
        snapshot: Some(snapshot),
    })
}

pub fn encode_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&Frame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority: AuthorityState::Unavailable,
        selected_present: None,
        snapshot: None,
    })
}

pub fn validate_snapshot(snapshot: &Snapshot) -> Result<(), CarrierError> {
    validate_snapshot_inner(snapshot, true)
}

/// Validate the completed semantic callback snapshot before the common GUI
/// bridge supplies selected-Present operation ownership. Operation vectors
/// must still be empty at this boundary; partially populated or guessed
/// mappings are forbidden.
pub fn validate_semantic_snapshot(snapshot: &Snapshot) -> Result<(), CarrierError> {
    validate_snapshot_inner(snapshot, false)
}

fn validate_snapshot_inner(
    snapshot: &Snapshot,
    require_selected_present_operations: bool,
) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0
        || !snapshot.producer_complete
        || !valid_tag(&snapshot.country_tag)
        || !valid_text(&snapshot.view_id)
        || !valid_text(&snapshot.producer_record_identity)
        || snapshot.visible_items.len() > MAX_VISIBLE_ITEMS
        || snapshot.expected_visible_item_count as usize != snapshot.visible_items.len()
        || snapshot.view_id != format!("nationalfocusview:{}", snapshot.country_tag)
        || snapshot.selected_present_canvas_operation_ordinals.len() > MAX_UI_PASS_OPERATIONS
        || snapshot.selected_present_ui_operation_order.len() > MAX_UI_PASS_OPERATIONS
    {
        return invalid("NFP1 snapshot identity/completeness invalid");
    }
    let mut ids = BTreeSet::new();
    let mut mapped_operations = BTreeSet::new();
    for (ordinal, item) in snapshot.visible_items.iter().enumerate() {
        if item.source_ordinal as usize != ordinal
            || !valid_focus_id(&item.focus_id)
            || !ids.insert(item.focus_id.as_str())
            || item.presentation_state_code > 3
            || item.display_progress_percent > 100
            || item.selected_present_ui_operation_ordinals.len() > MAX_UI_PASS_OPERATIONS
            || item
                .selected_present_ui_operation_ordinals
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || item
                .joint_viewing_country_tag
                .as_deref()
                .is_some_and(|tag| !valid_tag(tag))
        {
            return invalid("NFP1 item identity/order/domain invalid");
        }
        if require_selected_present_operations
            != !item.selected_present_ui_operation_ordinals.is_empty()
        {
            return invalid("NFP1 focus operation ownership is absent or populated before its bridge");
        }
        if require_selected_present_operations
            && (item.selected_present_screen_rect.width <= 0
                || item.selected_present_screen_rect.height <= 0)
            || !require_selected_present_operations
                && item.selected_present_screen_rect != RectI32::EMPTY
        {
            return invalid("NFP1 focus selected-Present rectangle is absent or premature");
        }
        match item.presentation_state_code {
            0 | 2 if item.display_progress_percent != 0 => {
                return invalid("NFP1 not-started item carries progress")
            }
            1 if item.display_progress_percent != 100 => {
                return invalid("NFP1 completed item does not carry 100 percent")
            }
            _ => {}
        }
        for operation_ordinal in &item.selected_present_ui_operation_ordinals {
            if !mapped_operations.insert(*operation_ordinal) {
                return invalid("NFP1 selected-Present operation is assigned to multiple focuses");
            }
            if mapped_operations.len() > MAX_UI_PASS_OPERATIONS {
                return invalid("NFP1 selected-Present operation union exceeds bound");
            }
        }
    }
    if snapshot
        .selected_present_canvas_operation_ordinals
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return invalid("NFP1 canvas-level selected-Present operations are unordered");
    }
    for operation_ordinal in &snapshot.selected_present_canvas_operation_ordinals {
        if !mapped_operations.insert(*operation_ordinal) {
            return invalid("NFP1 selected-Present operation has multiple canvas owners");
        }
    }
    if require_selected_present_operations
        && (snapshot.selected_present_ui_operation_order.is_empty()
            || snapshot
                .selected_present_ui_operation_order
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || snapshot.selected_present_ui_operation_order
                != mapped_operations.iter().copied().collect::<Vec<_>>())
        || !require_selected_present_operations
            && (!snapshot.selected_present_canvas_operation_ordinals.is_empty()
                || !snapshot.selected_present_ui_operation_order.is_empty()
                || !mapped_operations.is_empty())
    {
        return invalid("NFP1 selected-Present operation order is not the complete focus union");
    }
    Ok(())
}

pub fn validate_frame(frame: &Frame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::Unavailable
            if frame.status_flags != 0
                && frame.selected_present.is_none()
                && frame.snapshot.is_none() =>
        {
            Ok(())
        }
        AuthorityState::Exact20Receipt if frame.status_flags == 0 => {
            let selected_present = frame.selected_present.as_ref().ok_or_else(|| {
                CarrierError::Invalid("NFP1 exact frame omitted selected-Present binding".into())
            })?;
            if selected_present.materialization_qpc_ticks == 0
                || selected_present.selected_present_global_order == 0
                || selected_present.selected_present_candidate_id == 0
                || selected_present.selected_present_qpc_ticks == 0
                || selected_present.selected_present_cutoff_qpc_ticks
                    < selected_present.selected_present_qpc_ticks
                || selected_present.qpc_frequency_hz == 0
                || selected_present.materialization_qpc_ticks
                    > selected_present.selected_present_qpc_ticks
                || !valid_sha256(&selected_present.selected_present_receipt_sha256)
                || !valid_sha256(
                    &selected_present.selected_present_pass_identity_sha256,
                )
                || selected_present.selected_present_engine_instance_token == 0
                || selected_present.authored_surface_identity != AUTHORED_SURFACE_IDENTITY
                || selected_present.runtime_root_identity != RUNTIME_ROOT_IDENTITY
                || selected_present.effective_interface_winner
                    != EFFECTIVE_INTERFACE_WINNER
                || selected_present
                    .render_target_extent_pixels
                    .iter()
                    .any(|value| *value <= 0)
                || selected_present
                    .client_extent_pixels
                    .iter()
                    .any(|value| *value <= 0)
                || selected_present
                    .content_extent_pixels
                    .iter()
                    .any(|value| *value <= 0)
                || selected_present
                    .viewport_extent_pixels
                    .iter()
                    .any(|value| *value <= 0)
                || selected_present.clip_rect.width <= 0
                || selected_present.clip_rect.height <= 0
                || selected_present.canvas_scale_bits.iter().any(|bits| {
                    let scale = f32::from_bits(*bits);
                    !scale.is_finite() || scale <= 0.0
                })
            {
                return invalid("NFP1 selected-Present binding is incomplete or unordered");
            }
            let clip_right = selected_present
                .clip_rect
                .x
                .checked_add(selected_present.clip_rect.width);
            let clip_bottom = selected_present
                .clip_rect
                .y
                .checked_add(selected_present.clip_rect.height);
            if selected_present.clip_rect.x < 0
                || selected_present.clip_rect.y < 0
                || clip_right.is_none_or(|right| {
                    right > selected_present.render_target_extent_pixels[0]
                        || right > selected_present.client_extent_pixels[0]
                })
                || clip_bottom.is_none_or(|bottom| {
                    bottom > selected_present.render_target_extent_pixels[1]
                        || bottom > selected_present.client_extent_pixels[1]
                })
                || selected_present.viewport_extent_pixels[0]
                    > selected_present.content_extent_pixels[0]
                || selected_present.viewport_extent_pixels[1]
                    > selected_present.content_extent_pixels[1]
                || selected_present.viewport_extent_pixels[0]
                    > selected_present.client_extent_pixels[0]
                || selected_present.viewport_extent_pixels[1]
                    > selected_present.client_extent_pixels[1]
            {
                return invalid("NFP1 canvas geometry is outside its exact target/content bounds");
            }
            let snapshot = frame
                .snapshot
                .as_ref()
                .ok_or_else(|| CarrierError::Invalid("NFP1 exact frame omitted snapshot".into()))?;
            validate_snapshot(snapshot)?;
            let expected_identity = format!(
                "a729:a31:{}:{}:{}",
                frame.frame_hour, snapshot.country_tag, snapshot.source_sequence
            );
            if snapshot.producer_record_identity != expected_identity {
                return invalid("NFP1 producer record identity disagrees with frame");
            }
            Ok(())
        }
        _ => invalid("NFP1 authority/status/snapshot mismatch"),
    }
}

pub fn encode_payload(frame: &Frame) -> Result<Vec<u8>, CarrierError> {
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
    output.extend_from_slice(&EXACT20_JOB_RECEIPT_SHA256);
    output.push(u8::from(frame.selected_present.is_some()));
    output.extend_from_slice(&[0; 3]);
    if let Some(selected) = &frame.selected_present {
        u64v(&mut output, selected.materialization_qpc_ticks);
        u32v(&mut output, selected.selected_present_global_order);
        u32v(&mut output, selected.selected_present_ordinal);
        u64v(&mut output, selected.selected_present_candidate_id);
        u64v(&mut output, selected.selected_present_qpc_ticks);
        u64v(
            &mut output,
            selected.selected_present_cutoff_qpc_ticks,
        );
        u64v(&mut output, selected.qpc_frequency_hz);
        output.extend_from_slice(
            &decode_sha256(&selected.selected_present_receipt_sha256)
                .expect("validated selected-Present SHA-256"),
        );
        output.extend_from_slice(
            &decode_sha256(&selected.selected_present_pass_identity_sha256)
                .expect("validated selected-Present pass SHA-256"),
        );
        u64v(
            &mut output,
            selected.selected_present_engine_instance_token,
        );
        textv(&mut output, &selected.authored_surface_identity);
        textv(&mut output, &selected.runtime_root_identity);
        textv(&mut output, &selected.effective_interface_winner);
        for value in selected.render_target_extent_pixels {
            i32v(&mut output, value);
        }
        for value in selected.client_extent_pixels {
            i32v(&mut output, value);
        }
        rectv(&mut output, selected.clip_rect);
        for value in selected.content_extent_pixels {
            i32v(&mut output, value);
        }
        for value in selected.viewport_extent_pixels {
            i32v(&mut output, value);
        }
        for value in selected.canvas_origin_pixels_fixed5 {
            i64v(&mut output, value);
        }
        for value in selected.pan_pixels_fixed5 {
            i64v(&mut output, value);
        }
        for value in selected.canvas_scale_bits {
            u32v(&mut output, value);
        }
    }
    output.push(u8::from(frame.snapshot.is_some()));
    output.extend_from_slice(&[0; 3]);
    if let Some(snapshot) = &frame.snapshot {
        u64v(&mut output, snapshot.source_sequence);
        textv(&mut output, &snapshot.country_tag);
        textv(&mut output, &snapshot.view_id);
        textv(&mut output, &snapshot.producer_record_identity);
        u32v(&mut output, snapshot.expected_visible_item_count);
        u32v(&mut output, snapshot.visible_items.len() as u32);
        for item in &snapshot.visible_items {
            u32v(&mut output, item.source_ordinal);
            output.push(item.presentation_state_code);
            output.push(item.display_progress_percent);
            let mut flags = 0;
            if item.highlight_glow_visible {
                flags |= ITEM_FLAG_HIGHLIGHT;
            }
            if item.historical_marker_visible {
                flags |= ITEM_FLAG_HISTORICAL;
            }
            if item.joint_viewing_country_tag.is_some() {
                flags |= ITEM_FLAG_JOINT_COUNTRY;
            }
            u16v(&mut output, flags);
            textv(&mut output, &item.focus_id);
            opt_textv(&mut output, item.joint_viewing_country_tag.as_deref());
            u32v(
                &mut output,
                item.selected_present_ui_operation_ordinals.len() as u32,
            );
            for operation_ordinal in &item.selected_present_ui_operation_ordinals {
                u32v(&mut output, *operation_ordinal);
            }
            rectv(&mut output, item.selected_present_screen_rect);
        }
        u32v(
            &mut output,
            snapshot.selected_present_canvas_operation_ordinals.len() as u32,
        );
        for operation_ordinal in &snapshot.selected_present_canvas_operation_ordinals {
            u32v(&mut output, *operation_ordinal);
        }
        u32v(
            &mut output,
            snapshot.selected_present_ui_operation_order.len() as u32,
        );
        for operation_ordinal in &snapshot.selected_present_ui_operation_order {
            u32v(&mut output, *operation_ordinal);
        }
    }
    Ok(output)
}

pub fn decode_payload(payload: &[u8]) -> Result<Frame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("NFP1 magic/schema mismatch");
    }
    let authority = match reader.u32()? {
        0 => AuthorityState::Unavailable,
        1 => AuthorityState::Exact20Receipt,
        _ => return invalid("NFP1 authority enum unknown"),
    };
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    if reader.bytes(32)? != EXECUTABLE_SHA256
        || reader.bytes(32)? != EXACT20_FUNCTION_MANIFEST_SHA256
        || reader.bytes(32)? != EXACT20_JOB_RECEIPT_SHA256
    {
        return invalid("NFP1 executable/receipt identity mismatch");
    }
    let selected_present = match reader.u8()? {
        0 => {
            if reader.bytes(3)? != [0, 0, 0] {
                return invalid("NFP1 selected-Present reserved bytes invalid");
            }
            None
        }
        1 => {
            if reader.bytes(3)? != [0, 0, 0] {
                return invalid("NFP1 selected-Present reserved bytes invalid");
            }
            let materialization_qpc_ticks = reader.u64()?;
            let selected_present_global_order = reader.u32()?;
            let selected_present_ordinal = reader.u32()?;
            let selected_present_candidate_id = reader.u64()?;
            let selected_present_qpc_ticks = reader.u64()?;
            let selected_present_cutoff_qpc_ticks = reader.u64()?;
            let qpc_frequency_hz = reader.u64()?;
            let selected_present_receipt_sha256 = encode_hex(reader.bytes(32)?);
            let selected_present_pass_identity_sha256 = encode_hex(reader.bytes(32)?);
            let selected_present_engine_instance_token = reader.u64()?;
            let authored_surface_identity = reader.text()?;
            let runtime_root_identity = reader.text()?;
            let effective_interface_winner = reader.text()?;
            let render_target_extent_pixels = [reader.i32()?, reader.i32()?];
            let client_extent_pixels = [reader.i32()?, reader.i32()?];
            let clip_rect = reader.rect()?;
            let content_extent_pixels = [reader.i32()?, reader.i32()?];
            let viewport_extent_pixels = [reader.i32()?, reader.i32()?];
            let canvas_origin_pixels_fixed5 = [reader.i64()?, reader.i64()?];
            let pan_pixels_fixed5 = [reader.i64()?, reader.i64()?];
            let canvas_scale_bits = [reader.u32()?, reader.u32()?];
            Some(SelectedPresentBinding {
                materialization_qpc_ticks,
                selected_present_global_order,
                selected_present_candidate_id,
                selected_present_ordinal,
                selected_present_qpc_ticks,
                selected_present_cutoff_qpc_ticks,
                qpc_frequency_hz,
                selected_present_receipt_sha256,
                selected_present_pass_identity_sha256,
                selected_present_engine_instance_token,
                authored_surface_identity,
                runtime_root_identity,
                effective_interface_winner,
                render_target_extent_pixels,
                client_extent_pixels,
                clip_rect,
                content_extent_pixels,
                viewport_extent_pixels,
                canvas_origin_pixels_fixed5,
                pan_pixels_fixed5,
                canvas_scale_bits,
            })
        }
        _ => return invalid("NFP1 selected-Present marker invalid"),
    };
    let present = reader.u8()?;
    if present > 1 || reader.bytes(3)? != [0, 0, 0] {
        return invalid("NFP1 snapshot marker/reserved bytes invalid");
    }
    let snapshot = if present == 1 {
        let source_sequence = reader.u64()?;
        let country_tag = reader.text()?;
        let view_id = reader.text()?;
        let producer_record_identity = reader.text()?;
        let expected_visible_item_count = reader.u32()?;
        let count = reader.u32()? as usize;
        if count > MAX_VISIBLE_ITEMS {
            return invalid("NFP1 item count exceeds bound");
        }
        let mut visible_items = Vec::with_capacity(count);
        let mut decoded_owned_operation_count = 0usize;
        for _ in 0..count {
            let source_ordinal = reader.u32()?;
            let presentation_state_code = reader.u8()?;
            let display_progress_percent = reader.u8()?;
            let flags = reader.u16()?;
            if flags & !ITEM_KNOWN_FLAGS != 0 {
                return invalid("NFP1 item carries unknown flags");
            }
            let focus_id = reader.text()?;
            let joint_viewing_country_tag = reader.opt_text()?;
            if (flags & ITEM_FLAG_JOINT_COUNTRY != 0) != joint_viewing_country_tag.is_some() {
                return invalid("NFP1 joint-country flag/value mismatch");
            }
            let operation_count = reader.u32()? as usize;
            if operation_count == 0 || operation_count > MAX_UI_PASS_OPERATIONS {
                return invalid("NFP1 focus operation count exceeds bound or is empty");
            }
            decoded_owned_operation_count = decoded_owned_operation_count
                .checked_add(operation_count)
                .filter(|count| *count <= MAX_UI_PASS_OPERATIONS)
                .ok_or_else(|| {
                    CarrierError::Invalid(
                        "NFP1 decoded per-focus operation total exceeds bound".into(),
                    )
                })?;
            let mut selected_present_ui_operation_ordinals =
                Vec::with_capacity(operation_count);
            for _ in 0..operation_count {
                selected_present_ui_operation_ordinals.push(reader.u32()?);
            }
            let selected_present_screen_rect = reader.rect()?;
            visible_items.push(FocusPresentationItem {
                focus_id,
                source_ordinal,
                presentation_state_code,
                display_progress_percent,
                highlight_glow_visible: flags & ITEM_FLAG_HIGHLIGHT != 0,
                historical_marker_visible: flags & ITEM_FLAG_HISTORICAL != 0,
                joint_viewing_country_tag,
                selected_present_ui_operation_ordinals,
                selected_present_screen_rect,
            });
        }
        let canvas_operation_count = reader.u32()? as usize;
        if canvas_operation_count > MAX_UI_PASS_OPERATIONS {
            return invalid("NFP1 canvas operation count exceeds bound");
        }
        decoded_owned_operation_count = decoded_owned_operation_count
            .checked_add(canvas_operation_count)
            .filter(|count| *count <= MAX_UI_PASS_OPERATIONS)
            .ok_or_else(|| {
                CarrierError::Invalid(
                    "NFP1 decoded root-owned operation total exceeds bound".into(),
                )
            })?;
        let mut selected_present_canvas_operation_ordinals =
            Vec::with_capacity(canvas_operation_count);
        for _ in 0..canvas_operation_count {
            selected_present_canvas_operation_ordinals.push(reader.u32()?);
        }
        let operation_count = reader.u32()? as usize;
        if operation_count == 0 || operation_count > MAX_UI_PASS_OPERATIONS {
            return invalid("NFP1 complete operation union exceeds bound or is empty");
        }
        if operation_count != decoded_owned_operation_count {
            return invalid("NFP1 decoded operation union count differs from owned total");
        }
        let mut selected_present_ui_operation_order = Vec::with_capacity(operation_count);
        for _ in 0..operation_count {
            selected_present_ui_operation_order.push(reader.u32()?);
        }
        Some(Snapshot {
            source_sequence,
            producer_complete: true,
            country_tag,
            view_id,
            producer_record_identity,
            expected_visible_item_count,
            visible_items,
            selected_present_canvas_operation_ordinals,
            selected_present_ui_operation_order,
        })
    } else {
        None
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = Frame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority,
        selected_present,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn valid_tag(value: &str) -> bool {
    value.len() == 3
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && !value.as_bytes().contains(&0)
        && !value.chars().any(char::is_control)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if !valid_sha256(value) {
        return None;
    }
    let mut output = [0u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        let pair = value.as_bytes().get(index * 2..index * 2 + 2)?;
        *byte = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(output)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

/// Portable Clausewitz focus-definition identity. The engine definition
/// pointer is resolved before this boundary; host paths, pointer spellings,
/// and path separators are never valid substitutes for the authored token.
/// Dots, dashes, and `@` remain available to mods in addition to the usual
/// ASCII alphanumeric/underscore spelling.
pub fn valid_focus_id(value: &str) -> bool {
    const MAX_FOCUS_ID_BYTES: usize = 128;
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_FOCUS_ID_BYTES
        || !(bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_')
        || value.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'@'))
        })
        || value.contains("..")
    {
        return false;
    }
    let pointer_digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    !pointer_digits.is_some_and(|digits| {
        digits.len() >= 8 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}

fn u16v(output: &mut Vec<u8>, value: u16) { output.extend_from_slice(&value.to_le_bytes()); }
fn u32v(output: &mut Vec<u8>, value: u32) { output.extend_from_slice(&value.to_le_bytes()); }
fn u64v(output: &mut Vec<u8>, value: u64) { output.extend_from_slice(&value.to_le_bytes()); }
fn i32v(output: &mut Vec<u8>, value: i32) { output.extend_from_slice(&value.to_le_bytes()); }
fn i64v(output: &mut Vec<u8>, value: i64) { output.extend_from_slice(&value.to_le_bytes()); }
fn rectv(output: &mut Vec<u8>, value: RectI32) {
    i32v(output, value.x);
    i32v(output, value.y);
    i32v(output, value.width);
    i32v(output, value.height);
}
fn textv(output: &mut Vec<u8>, value: &str) { u32v(output, value.len() as u32); output.extend_from_slice(value.as_bytes()); }
fn opt_textv(output: &mut Vec<u8>, value: Option<&str>) { match value { Some(value) => { output.push(1); textv(output, value); }, None => output.push(0), } }

struct Reader<'a> { payload: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn bytes(&mut self, count: usize) -> Result<&'a [u8], CarrierError> { let end = self.at.checked_add(count).ok_or(CarrierError::Truncated)?; let bytes = self.payload.get(self.at..end).ok_or(CarrierError::Truncated)?; self.at = end; Ok(bytes) }
    fn u8(&mut self) -> Result<u8, CarrierError> { Ok(self.bytes(1)?[0]) }
    fn u16(&mut self) -> Result<u16, CarrierError> { Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> Result<u32, CarrierError> { Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64, CarrierError> { Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap())) }
    fn i32(&mut self) -> Result<i32, CarrierError> { Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap())) }
    fn i64(&mut self) -> Result<i64, CarrierError> { Ok(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap())) }
    fn rect(&mut self) -> Result<RectI32, CarrierError> { Ok(RectI32 { x: self.i32()?, y: self.i32()?, width: self.i32()?, height: self.i32()? }) }
    fn text(&mut self) -> Result<String, CarrierError> { let count = self.u32()? as usize; if count > MAX_TEXT_BYTES { return invalid("NFP1 text exceeds bound"); } String::from_utf8(self.bytes(count)?.to_vec()).map_err(|_| CarrierError::Invalid("NFP1 text is not UTF-8".into())) }
    fn opt_text(&mut self) -> Result<Option<String>, CarrierError> { match self.u8()? { 0 => Ok(None), 1 => Ok(Some(self.text()?)), _ => invalid("NFP1 optional-text marker invalid"), } }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selected_present() -> SelectedPresentBinding {
        SelectedPresentBinding {
            materialization_qpc_ticks: 90,
            selected_present_global_order: 7,
            selected_present_candidate_id: 3,
            selected_present_ordinal: 0,
            selected_present_qpc_ticks: 100,
            selected_present_cutoff_qpc_ticks: 110,
            qpc_frequency_hz: 10_000_000,
            selected_present_receipt_sha256: "ab".repeat(32),
            selected_present_pass_identity_sha256: "cd".repeat(32),
            selected_present_engine_instance_token: 17,
            authored_surface_identity: AUTHORED_SURFACE_IDENTITY.into(),
            runtime_root_identity: RUNTIME_ROOT_IDENTITY.into(),
            effective_interface_winner: EFFECTIVE_INTERFACE_WINNER.into(),
            render_target_extent_pixels: [1920, 1080],
            client_extent_pixels: [1920, 1080],
            clip_rect: RectI32 { x: 100, y: 100, width: 1200, height: 800 },
            content_extent_pixels: [2000, 1600],
            viewport_extent_pixels: [1200, 800],
            canvas_origin_pixels_fixed5: [3200, 3200],
            pan_pixels_fixed5: [-640, 960],
            canvas_scale_bits: [1.0f32.to_bits(), 1.0f32.to_bits()],
        }
    }
    fn frame() -> Frame { Frame { elapsed_wall_ms: 7, frame_hour: 42, status_flags: 0, authority: AuthorityState::Exact20Receipt, selected_present: Some(selected_present()), snapshot: Some(Snapshot { source_sequence: 9, producer_complete: true, country_tag: "GER".into(), view_id: "nationalfocusview:GER".into(), producer_record_identity: "a729:a31:42:GER:9".into(), expected_visible_item_count: 2, visible_items: vec![ FocusPresentationItem { focus_id: "ger_a".into(), source_ordinal: 0, presentation_state_code: 1, display_progress_percent: 100, highlight_glow_visible: false, historical_marker_visible: true, joint_viewing_country_tag: None, selected_present_ui_operation_ordinals: vec![4, 5], selected_present_screen_rect: RectI32 { x: 120, y: 140, width: 80, height: 80 } }, FocusPresentationItem { focus_id: "ger_b".into(), source_ordinal: 1, presentation_state_code: 3, display_progress_percent: 37, highlight_glow_visible: true, historical_marker_visible: false, joint_viewing_country_tag: Some("ITA".into()), selected_present_ui_operation_ordinals: vec![6], selected_present_screen_rect: RectI32 { x: 240, y: 260, width: 80, height: 80 } } ], selected_present_canvas_operation_ordinals: vec![2, 3], selected_present_ui_operation_order: vec![2, 3, 4, 5, 6] }) } }
    #[test] fn roundtrip_preserves_pointer_free_item_identity_and_engine_order() { let expected = frame(); assert_eq!(decode_payload(&encode_payload(&expected).unwrap()).unwrap(), expected); }
    #[test] fn incomplete_or_inferred_state_rejects_whole_frame() { let mut value = frame(); value.snapshot.as_mut().unwrap().visible_items[1].source_ordinal = 7; assert!(encode_payload(&value).is_err()); let mut value = frame(); value.snapshot.as_mut().unwrap().visible_items[0].display_progress_percent = 99; assert!(encode_payload(&value).is_err()); }
    #[test] fn production_activation_uses_exact_source_without_fixture_gate() { struct Source; impl ExactNationalFocusPresentationSource for Source { fn is_exact_a729_focus_view_source(&self) -> bool { true } fn producer_manifest_sha256(&self) -> [u8; 32] { EXACT20_FUNCTION_MANIFEST_SHA256 } fn collect_snapshot(&self, _: u64, _: &str) -> Result<Snapshot, String> { Ok(frame().snapshot.unwrap()) } } assert!(collect_and_encode(&Source, 42, 7, "GER", selected_present()).is_ok()); }
    #[test]
    fn selected_present_binding_is_mandatory_and_ordered() {
        let mut value = frame();
        value.selected_present = None;
        assert!(encode_payload(&value).is_err());
        let mut value = frame();
        value.selected_present.as_mut().unwrap().materialization_qpc_ticks = 101;
        assert!(encode_payload(&value).is_err());
        let mut value = frame();
        value.selected_present.as_mut().unwrap().selected_present_receipt_sha256 = "A".repeat(64);
        assert!(encode_payload(&value).is_err());
    }
    #[test]
    fn source_authority_and_requested_country_are_mandatory() {
        struct Source { authenticated: bool, manifest: [u8; 32], country: &'static str }
        impl ExactNationalFocusPresentationSource for Source {
            fn is_exact_a729_focus_view_source(&self) -> bool { self.authenticated }
            fn producer_manifest_sha256(&self) -> [u8; 32] { self.manifest }
            fn collect_snapshot(&self, _: u64, _: &str) -> Result<Snapshot, String> {
                let mut snapshot = frame().snapshot.unwrap();
                snapshot.country_tag = self.country.into();
                snapshot.view_id = format!("nationalfocusview:{}", self.country);
                snapshot.producer_record_identity = format!("a729:a31:42:{}:9", self.country);
                Ok(snapshot)
            }
        }
        assert_eq!(collect_and_validate(&Source { authenticated: false, manifest: EXACT20_FUNCTION_MANIFEST_SHA256, country: "GER" }, 42, "GER"), Err(CarrierError::UnauthenticatedSource));
        assert_eq!(collect_and_validate(&Source { authenticated: true, manifest: [7; 32], country: "GER" }, 42, "GER"), Err(CarrierError::UnauthenticatedSource));
        assert!(collect_and_validate(&Source { authenticated: true, manifest: EXACT20_FUNCTION_MANIFEST_SHA256, country: "ITA" }, 42, "GER").is_err());
        assert!(collect_and_validate(&Source { authenticated: true, manifest: EXACT20_FUNCTION_MANIFEST_SHA256, country: "GER" }, 42, "GER").is_ok());
    }
    #[test]
    fn focus_identity_is_portable_and_mod_valid() {
        for accepted in ["GER_autobahn", "mod-focus.v2", "shared@branch", "_joint_focus"] {
            assert!(valid_focus_id(accepted), "{accepted}");
        }
        for rejected in [
            "0x000001abcdef1234",
            r"C:\\hoi4\\focus",
            "/opt/hoi4/focus",
            r"\\host\\share\\focus",
            "focus/child",
            "focus..child",
            "focus child",
        ] {
            assert!(!valid_focus_id(rejected), "{rejected}");
            let mut value = frame();
            value.snapshot.as_mut().unwrap().visible_items[0].focus_id = rejected.into();
            assert!(encode_payload(&value).is_err(), "{rejected}");
        }
    }
}
