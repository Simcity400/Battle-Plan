//! XUI21 exact fired-event producer carrier (d75).
//!
//! Only the authenticated a729 inline-hook runtime may construct a production
//! frame. Save ledgers and delayed-event polling deliberately cannot implement
//! [`ExactFiredEventSource`].

use std::collections::BTreeSet;
use std::sync::Mutex;

#[cfg(windows)]
pub mod a729_bridge;
#[cfg(windows)]
pub mod a729_inline_hook;
#[cfg(windows)]
pub mod a729_runtime;

pub const DOMAIN_ID: u16 = 75;
pub const DOMAIN_NAME: &str = "fired_event_popups_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"FEV1");
/// v2 replaces the ambiguous singular runtime-text join with independently
/// authenticated title, description, and definition-ordinal option joins.
pub const SCHEMA_VERSION: u32 = 2;
pub const AUTHORITY_A729_INLINE_HOOKS: u32 = 1;
pub const MAX_ROWS: usize = 1_000_000;
pub const MAX_OPTIONS: usize = 65_536;
pub const MAX_STRING: usize = 65_536;
pub const PRODUCER_MANIFEST_SHA256: [u8; 32] = [
    0x24, 0x0d, 0x75, 0x00, 0xbc, 0xc9, 0x15, 0x14, 0xf5, 0x9d, 0x62, 0x73, 0x9d, 0xf0, 0x26, 0xa5,
    0xa7, 0x6c, 0xce, 0x41, 0xf8, 0x24, 0x92, 0x34, 0x22, 0xcb, 0xe2, 0x53, 0xe0, 0x5d, 0x1c, 0xec,
];
/// Exact-a729 read-only Ghidra export manifest that certifies the common fire
/// producer plus the player, AI, timeout-default, and timeout-no-choice hook
/// routes shared by all five popup classes. Keeping this as a second pin makes
/// producer-manifest drift fail closed instead of silently changing the static
/// route authority.
pub const CERTIFIED_ROUTE_MANIFEST_SHA256: [u8; 32] = [
    0x24, 0x0d, 0x75, 0x00, 0xbc, 0xc9, 0x15, 0x14, 0xf5, 0x9d, 0x62, 0x73, 0x9d, 0xf0, 0x26, 0xa5,
    0xa7, 0x6c, 0xce, 0x41, 0xf8, 0x24, 0x92, 0x34, 0x22, 0xcb, 0xe2, 0x53, 0xe0, 0x5d, 0x1c, 0xec,
];
pub const CERTIFIED_ROUTE_RECEIPT: &str = "replay_interface/re_evidence/rendering_laws/XUI21/H_POPUP_FULL_GRAPH_GHIDRA_RECEIPT_20260720.json";
pub const CERTIFIED_ROUTE_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupSubtype {
    Country,
    News,
    State,
    UnitLeader,
    OperativeLeader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionReason {
    Pending,
    Selected,
    Timeout,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionRuntimeTextContext {
    /// Definition-array ordinal, never the popup-visible position.
    pub definition_ordinal: u16,
    /// Exact join key into A25. Process-local textbox owners never cross wire.
    pub context_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiredEventRow {
    pub event_id: String,
    pub recipient_tag: String,
    pub fire_date: String,
    pub fire_hour: u64,
    pub fire_tick: u64,
    pub event_sequence: u64,
    pub visible_option_ordinals: Vec<u16>,
    pub chosen_option_ordinal: Option<u16>,
    pub selected_desc_key: Option<String>,
    pub shown_picture_gfx: Option<String>,
    pub popup_subtype: PopupSubtype,
    pub effective_timeout_hours: u32,
    pub timeout_tick: u64,
    pub resolved_tick: Option<u64>,
    pub resolution_reason: ResolutionReason,
    /// Independent A25 joins are required because title, description, and each
    /// visible option are distinct textbox owners with distinct source tokens.
    pub runtime_title_text_context_id: Option<String>,
    pub runtime_description_text_context_id: Option<String>,
    pub runtime_option_text_contexts: Vec<OptionRuntimeTextContext>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookTelemetry {
    pub fire_hook_validated: bool,
    pub option_hook_validated: bool,
    pub materialisation_hook_validated: bool,
    pub timeout_hook_validated: bool,
    /// Wire-compatible name for exact-build popup-class route certification.
    /// This does not claim every class occurred in the current capture.
    pub all_event_classes_covered: bool,
    /// Wire-compatible name for exact-build terminal-route certification.
    /// This does not claim all four paths occurred in the current capture.
    pub player_ai_timeout_no_choice_covered: bool,
    pub capture_start_baseline_complete: bool,
    pub queue_capacity: u32,
    pub queue_high_water_mark: u32,
    pub dropped_fire_rows: u64,
    pub dropped_resolution_rows: u64,
    pub correlation_failures: u64,
    pub invalid_definition_joins: u64,
    pub fired_ledger_shadow_mismatches: u64,
}

impl HookTelemetry {
    pub fn zero_gap(&self) -> bool {
        self.fire_hook_validated
            && self.option_hook_validated
            && self.materialisation_hook_validated
            && self.timeout_hook_validated
            && self.all_event_classes_covered
            && self.player_ai_timeout_no_choice_covered
            && self.capture_start_baseline_complete
            && self.queue_capacity > 0
            && self.queue_high_water_mark <= self.queue_capacity
            && self.dropped_fire_rows == 0
            && self.dropped_resolution_rows == 0
            && self.correlation_failures == 0
            && self.invalid_definition_joins == 0
            && self.fired_ledger_shadow_mismatches == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FiredEventFrame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub producer_manifest_sha256: [u8; 32],
    pub telemetry: HookTelemetry,
    pub fired_events_complete: bool,
    pub rows: Vec<FiredEventRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    UnauthenticatedSource,
    Invalid(String),
    Truncated,
    TrailingBytes,
    Source(String),
}

mod exact_source_seal {
    pub trait Sealed {}
}

/// Sealed producer boundary. Downstream crates may consume an installed source
/// but cannot implement this trait for a save ledger or d64 delayed-event poll.
pub trait ExactFiredEventSource: exact_source_seal::Sealed {
    fn is_authenticated_a729_hook_source(&self) -> bool;
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn snapshot(
        &self,
        frame_hour: u64,
        elapsed_wall_ms: u64,
    ) -> Result<(HookTelemetry, Vec<FiredEventRow>), String>;
}

/// Transactional ledger owned by the installed hook runtime. Callback writes
/// wait for the short ledger critical section instead of treating the owner's
/// cumulative snapshot clone as producer loss. The runtime epoch makes a
/// snapshot retry if any callback overlaps it.
/// Process-local callback ledger. This is deliberately crate-private: the only
/// production [`ExactFiredEventSource`] is the installed hook runtime. A save
/// ledger, delayed-event poller, or downstream crate cannot construct an
/// authenticated d75 source by filling this buffer directly.
pub(crate) struct A729EventLedger {
    state: Mutex<LedgerState>,
    capacity: usize,
}

struct LedgerState {
    telemetry: HookTelemetry,
    rows: Vec<FiredEventRow>,
}

#[derive(Clone, Debug)]
pub struct FireInput {
    pub event_id: String,
    pub recipient_tag: String,
    pub fire_date: String,
    pub fire_hour: u64,
    pub fire_tick: u64,
    pub event_sequence: u64,
    pub popup_subtype: PopupSubtype,
    pub effective_timeout_hours: u32,
    pub timeout_tick: u64,
}

#[derive(Clone, Debug)]
pub struct MaterialisationInput {
    pub event_sequence: u64,
    pub visible_option_ordinals: Vec<u16>,
    pub selected_desc_key: Option<String>,
    pub shown_picture_gfx: Option<String>,
    pub popup_subtype: PopupSubtype,
    pub timeout_tick: u64,
    pub runtime_title_text_context_id: Option<String>,
    pub runtime_description_text_context_id: Option<String>,
    pub runtime_option_text_contexts: Vec<OptionRuntimeTextContext>,
}

#[derive(Clone, Debug)]
pub struct ResolutionInput {
    pub event_sequence: u64,
    pub chosen_option_ordinal: Option<u16>,
    pub resolved_tick: u64,
    pub reason: ResolutionReason,
}

impl A729EventLedger {
    pub(crate) fn new(capacity: usize) -> Self {
        let capacity = capacity.clamp(1, MAX_ROWS);
        Self {
            state: Mutex::new(LedgerState {
                telemetry: HookTelemetry {
                    queue_capacity: capacity as u32,
                    ..Default::default()
                },
                rows: Vec::new(),
            }),
            capacity,
        }
    }

    /// Marks only the facts proved by successful installation of the complete
    /// pinned hook set. Coverage and baseline are separate witnesses.
    pub(crate) fn record_installation_validation(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.telemetry.fire_hook_validated = true;
            state.telemetry.option_hook_validated = true;
            state.telemetry.materialisation_hook_validated = true;
            state.telemetry.timeout_hook_validated = true;
        }
    }

    pub(crate) fn record_baseline_complete(&self, complete: bool) {
        if let Ok(mut state) = self.state.lock() {
            state.telemetry.capture_start_baseline_complete = complete;
        }
    }

    /// Records the separately pinned exact-build route certification. These
    /// bits describe hook-route authority, not branches exercised again in
    /// this capture. Per-capture completeness remains owned by the baseline,
    /// bounded queue, correlation counters, definition joins, and fired-ledger
    /// shadow below.
    pub(crate) fn record_route_certification(
        &self,
        all_classes: bool,
        all_resolution_paths: bool,
    ) {
        if let Ok(mut state) = self.state.lock() {
            state.telemetry.all_event_classes_covered = all_classes;
            state.telemetry.player_ai_timeout_no_choice_covered = all_resolution_paths;
        }
    }

    #[cfg(test)]
    pub fn authenticate_installation_and_route_certification(
        &self,
        baseline_complete: bool,
        all_classes: bool,
        all_resolution_paths: bool,
    ) {
        self.record_installation_validation();
        self.record_baseline_complete(baseline_complete);
        self.record_route_certification(all_classes, all_resolution_paths);
    }

    pub(crate) fn record_fire(&self, input: FireInput) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.rows.len() >= self.capacity
            || state
                .rows
                .iter()
                .any(|row| row.event_sequence == input.event_sequence)
        {
            state.telemetry.dropped_fire_rows = state.telemetry.dropped_fire_rows.saturating_add(1);
            return;
        }
        let row = FiredEventRow {
            event_id: input.event_id,
            recipient_tag: input.recipient_tag,
            fire_date: input.fire_date,
            fire_hour: input.fire_hour,
            fire_tick: input.fire_tick,
            event_sequence: input.event_sequence,
            visible_option_ordinals: Vec::new(),
            chosen_option_ordinal: None,
            selected_desc_key: None,
            shown_picture_gfx: None,
            popup_subtype: input.popup_subtype,
            effective_timeout_hours: input.effective_timeout_hours,
            timeout_tick: input.timeout_tick,
            resolved_tick: None,
            resolution_reason: ResolutionReason::Pending,
            runtime_title_text_context_id: None,
            runtime_description_text_context_id: None,
            runtime_option_text_contexts: Vec::new(),
        };
        let position = state
            .rows
            .partition_point(|existing| existing.event_sequence < row.event_sequence);
        state.rows.insert(position, row);
        state.telemetry.queue_high_water_mark = state
            .telemetry
            .queue_high_water_mark
            .max(state.rows.len() as u32);
    }

    pub(crate) fn record_materialisation(&self, input: MaterialisationInput) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let Some(row_index) = state
            .rows
            .iter()
            .position(|row| row.event_sequence == input.event_sequence)
        else {
            state.telemetry.correlation_failures =
                state.telemetry.correlation_failures.saturating_add(1);
            return;
        };
        let fire_tick = state.rows[row_index].fire_tick;
        let Some(effective_timeout_hours) = input
            .timeout_tick
            .checked_sub(fire_tick)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value != 0)
        else {
            state.telemetry.correlation_failures =
                state.telemetry.correlation_failures.saturating_add(1);
            return;
        };
        let row = &mut state.rows[row_index];
        row.visible_option_ordinals = input.visible_option_ordinals;
        row.selected_desc_key = input.selected_desc_key;
        row.shown_picture_gfx = input.shown_picture_gfx;
        row.popup_subtype = input.popup_subtype;
        row.effective_timeout_hours = effective_timeout_hours;
        row.timeout_tick = input.timeout_tick;
        row.runtime_title_text_context_id = input.runtime_title_text_context_id;
        row.runtime_description_text_context_id = input.runtime_description_text_context_id;
        row.runtime_option_text_contexts = input.runtime_option_text_contexts;
    }

    pub(crate) fn record_resolution(&self, input: ResolutionInput) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let Some(row) = state
            .rows
            .iter_mut()
            .find(|row| row.event_sequence == input.event_sequence)
        else {
            state.telemetry.correlation_failures =
                state.telemetry.correlation_failures.saturating_add(1);
            return;
        };
        row.chosen_option_ordinal = input.chosen_option_ordinal;
        row.resolved_tick = Some(input.resolved_tick);
        row.resolution_reason = input.reason;
    }

    pub(crate) fn note_invalid_definition_join(&self) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.telemetry.invalid_definition_joins =
            s.telemetry.invalid_definition_joins.saturating_add(1)
    }
    pub(crate) fn note_correlation_failure(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.telemetry.correlation_failures =
            state.telemetry.correlation_failures.saturating_add(1)
    }
    pub(crate) fn note_shadow_mismatch(&self) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.telemetry.fired_ledger_shadow_mismatches =
            s.telemetry.fired_ledger_shadow_mismatches.saturating_add(1)
    }
    pub(crate) fn snapshot_rows(
        &self,
        _frame_hour: u64,
        _elapsed_wall_ms: u64,
    ) -> Result<(HookTelemetry, Vec<FiredEventRow>), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "event ledger poisoned".to_string())?;
        Ok((state.telemetry.clone(), state.rows.clone()))
    }
}

// Unit tests exercise the bounded ledger independently. Production builds do
// not expose this trait implementation; only A729RuntimeSource can cross the
// authenticated source boundary.
#[cfg(test)]
impl exact_source_seal::Sealed for A729EventLedger {}

#[cfg(test)]
impl ExactFiredEventSource for A729EventLedger {
    fn is_authenticated_a729_hook_source(&self) -> bool {
        true
    }
    fn producer_manifest_sha256(&self) -> [u8; 32] {
        PRODUCER_MANIFEST_SHA256
    }
    fn snapshot(
        &self,
        frame_hour: u64,
        elapsed_wall_ms: u64,
    ) -> Result<(HookTelemetry, Vec<FiredEventRow>), String> {
        self.snapshot_rows(frame_hour, elapsed_wall_ms)
    }
}

pub fn collect_and_encode<S: ExactFiredEventSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    if !source.is_authenticated_a729_hook_source()
        || source.producer_manifest_sha256() != PRODUCER_MANIFEST_SHA256
    {
        return Err(CarrierError::UnauthenticatedSource);
    }
    let (telemetry, rows) = source
        .snapshot(frame_hour, elapsed_wall_ms)
        .map_err(CarrierError::Source)?;
    let frame = FiredEventFrame {
        elapsed_wall_ms,
        frame_hour,
        producer_manifest_sha256: source.producer_manifest_sha256(),
        fired_events_complete: telemetry.zero_gap(),
        telemetry,
        rows,
    };
    encode_payload(&frame)
}

pub fn validate_frame(frame: &FiredEventFrame) -> Result<(), CarrierError> {
    if frame.producer_manifest_sha256 != PRODUCER_MANIFEST_SHA256 {
        return invalid("producer manifest mismatch");
    }
    if frame.fired_events_complete != frame.telemetry.zero_gap() {
        return invalid("completeness witness disagrees with zero-gap telemetry");
    }
    if frame.rows.len() > MAX_ROWS {
        return invalid("row count exceeds bound");
    }
    let mut seq = None;
    let mut tick = None;
    let mut identities = BTreeSet::new();
    for row in &frame.rows {
        nonempty(&row.event_id, "event id")?;
        tag(&row.recipient_tag)?;
        nonempty(&row.fire_date, "fire date")?;
        if !identities.insert(row.event_sequence) {
            return invalid("duplicate event sequence");
        }
        if seq.is_some_and(|v| row.event_sequence <= v) {
            return invalid("event sequence not strictly increasing");
        }
        if tick.is_some_and(|v| row.fire_tick < v) {
            return invalid("fire tick regresses");
        }
        seq = Some(row.event_sequence);
        tick = Some(row.fire_tick);
        if row.visible_option_ordinals.len() > MAX_OPTIONS
            || row.visible_option_ordinals.windows(2).any(|w| w[0] >= w[1])
        {
            return invalid("visible option ordinals not strictly ordered");
        }
        // AI and hidden firings have an exact selected definition ordinal but
        // no popup materialisation list. Once any popup presentation has been
        // observed, however, the chosen ordinal must belong to that exact list.
        let presentation_materialised = !row.visible_option_ordinals.is_empty()
            || row.selected_desc_key.is_some()
            || row.shown_picture_gfx.is_some();
        if presentation_materialised
            && row
                .chosen_option_ordinal
                .is_some_and(|v| !row.visible_option_ordinals.contains(&v))
        {
            return invalid("chosen option is not a materialised ordinal");
        }
        opt_nonempty(row.selected_desc_key.as_deref(), "description key")?;
        opt_nonempty(row.shown_picture_gfx.as_deref(), "picture gfx")?;
        opt_nonempty(
            row.runtime_title_text_context_id.as_deref(),
            "runtime title text context",
        )?;
        opt_nonempty(
            row.runtime_description_text_context_id.as_deref(),
            "runtime description text context",
        )?;
        if row.runtime_option_text_contexts.len() > MAX_OPTIONS
            || row
                .runtime_option_text_contexts
                .windows(2)
                .any(|pair| pair[0].definition_ordinal >= pair[1].definition_ordinal)
        {
            return invalid("runtime option text contexts not strictly ordered");
        }
        for context in &row.runtime_option_text_contexts {
            nonempty(&context.context_id, "runtime option text context")?;
            if !row
                .visible_option_ordinals
                .contains(&context.definition_ordinal)
            {
                return invalid("runtime option text context is not a visible ordinal");
            }
        }
        if row.effective_timeout_hours == 0 || row.timeout_tick <= row.fire_tick {
            return invalid("timeout is not after fire");
        }
        match row.resolution_reason {
            ResolutionReason::Pending
                if row.resolved_tick.is_none() && row.chosen_option_ordinal.is_none() => {}
            ResolutionReason::Selected
                if row.resolved_tick.is_some() && row.chosen_option_ordinal.is_some() => {}
            // The authenticated timeout producer has two distinct outcomes:
            // it applies the first admitted definition option when one exists,
            // or closes with no choice when none is admitted.  Preserve the
            // former ordinal instead of collapsing both paths to `None`.
            ResolutionReason::Timeout if row.resolved_tick.is_some() => {}
            _ => return invalid("resolution fields disagree"),
        }
        if row.resolved_tick.is_some_and(|v| v < row.fire_tick) {
            return invalid("resolution tick precedes fire");
        }
    }
    Ok(())
}

pub fn encode_payload(frame: &FiredEventFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, AUTHORITY_A729_INLINE_HOOKS);
    u32v(&mut out, u32::from(frame.fired_events_complete));
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    out.extend_from_slice(&frame.producer_manifest_sha256);
    put_telemetry(&mut out, &frame.telemetry);
    u32v(&mut out, frame.rows.len() as u32);
    for row in &frame.rows {
        string(&mut out, &row.event_id)?;
        string(&mut out, &row.recipient_tag)?;
        string(&mut out, &row.fire_date)?;
        u64v(&mut out, row.fire_hour);
        u64v(&mut out, row.fire_tick);
        u64v(&mut out, row.event_sequence);
        u32v(&mut out, row.visible_option_ordinals.len() as u32);
        for value in &row.visible_option_ordinals {
            u32v(&mut out, u32::from(*value));
        }
        optional_u16(&mut out, row.chosen_option_ordinal);
        optional_string(&mut out, row.selected_desc_key.as_deref())?;
        optional_string(&mut out, row.shown_picture_gfx.as_deref())?;
        u32v(&mut out, subtype_code(row.popup_subtype));
        u32v(&mut out, row.effective_timeout_hours);
        u64v(&mut out, row.timeout_tick);
        optional_u64(&mut out, row.resolved_tick);
        u32v(&mut out, reason_code(row.resolution_reason));
        optional_string(&mut out, row.runtime_title_text_context_id.as_deref())?;
        optional_string(&mut out, row.runtime_description_text_context_id.as_deref())?;
        u32v(&mut out, row.runtime_option_text_contexts.len() as u32);
        for context in &row.runtime_option_text_contexts {
            u32v(&mut out, u32::from(context.definition_ordinal));
            string(&mut out, &context.context_id)?;
        }
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<FiredEventFrame, CarrierError> {
    let mut r = Reader {
        data: payload,
        at: 0,
    };
    if r.u32()? != MAGIC || r.u32()? != SCHEMA_VERSION || r.u32()? != AUTHORITY_A729_INLINE_HOOKS {
        return Err(CarrierError::UnauthenticatedSource);
    }
    let fired_events_complete = r.bool()?;
    let elapsed_wall_ms = r.u64()?;
    let frame_hour = r.u64()?;
    let producer_manifest_sha256 = r.array32()?;
    let telemetry = r.telemetry()?;
    let count = r.u32()? as usize;
    if count > MAX_ROWS {
        return invalid("row count exceeds bound");
    }
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let event_id = r.string()?;
        let recipient_tag = r.string()?;
        let fire_date = r.string()?;
        let fire_hour = r.u64()?;
        let fire_tick = r.u64()?;
        let event_sequence = r.u64()?;
        let n = r.u32()? as usize;
        if n > MAX_OPTIONS {
            return invalid("option count exceeds bound");
        }
        let mut visible_option_ordinals = Vec::with_capacity(n);
        for _ in 0..n {
            visible_option_ordinals.push(
                u16::try_from(r.u32()?)
                    .map_err(|_| CarrierError::Invalid("option ordinal exceeds u16".into()))?,
            );
        }
        let chosen_option_ordinal = r.optional_u16()?;
        let selected_desc_key = r.optional_string()?;
        let shown_picture_gfx = r.optional_string()?;
        let popup_subtype = subtype_from(r.u32()?)?;
        let effective_timeout_hours = r.u32()?;
        let timeout_tick = r.u64()?;
        let resolved_tick = r.optional_u64()?;
        let resolution_reason = reason_from(r.u32()?)?;
        let runtime_title_text_context_id = r.optional_string()?;
        let runtime_description_text_context_id = r.optional_string()?;
        let context_count = r.u32()? as usize;
        if context_count > MAX_OPTIONS {
            return invalid("runtime option text context count exceeds bound");
        }
        let mut runtime_option_text_contexts = Vec::with_capacity(context_count);
        for _ in 0..context_count {
            runtime_option_text_contexts.push(OptionRuntimeTextContext {
                definition_ordinal: u16::try_from(r.u32()?).map_err(|_| {
                    CarrierError::Invalid("runtime option text ordinal exceeds u16".into())
                })?,
                context_id: r.string()?,
            });
        }
        rows.push(FiredEventRow {
            event_id,
            recipient_tag,
            fire_date,
            fire_hour,
            fire_tick,
            event_sequence,
            visible_option_ordinals,
            chosen_option_ordinal,
            selected_desc_key,
            shown_picture_gfx,
            popup_subtype,
            effective_timeout_hours,
            timeout_tick,
            resolved_tick,
            resolution_reason,
            runtime_title_text_context_id,
            runtime_description_text_context_id,
            runtime_option_text_contexts,
        });
    }
    if r.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = FiredEventFrame {
        elapsed_wall_ms,
        frame_hour,
        producer_manifest_sha256,
        telemetry,
        fired_events_complete,
        rows,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn put_telemetry(o: &mut Vec<u8>, t: &HookTelemetry) {
    for v in [
        t.fire_hook_validated,
        t.option_hook_validated,
        t.materialisation_hook_validated,
        t.timeout_hook_validated,
        t.all_event_classes_covered,
        t.player_ai_timeout_no_choice_covered,
        t.capture_start_baseline_complete,
    ] {
        u32v(o, u32::from(v));
    }
    u32v(o, t.queue_capacity);
    u32v(o, t.queue_high_water_mark);
    for v in [
        t.dropped_fire_rows,
        t.dropped_resolution_rows,
        t.correlation_failures,
        t.invalid_definition_joins,
        t.fired_ledger_shadow_mismatches,
    ] {
        u64v(o, v);
    }
}
fn subtype_code(v: PopupSubtype) -> u32 {
    match v {
        PopupSubtype::Country => 0,
        PopupSubtype::News => 1,
        PopupSubtype::State => 2,
        PopupSubtype::UnitLeader => 3,
        PopupSubtype::OperativeLeader => 4,
    }
}
fn subtype_from(v: u32) -> Result<PopupSubtype, CarrierError> {
    match v {
        0 => Ok(PopupSubtype::Country),
        1 => Ok(PopupSubtype::News),
        2 => Ok(PopupSubtype::State),
        3 => Ok(PopupSubtype::UnitLeader),
        4 => Ok(PopupSubtype::OperativeLeader),
        _ => invalid("popup subtype invalid"),
    }
}
fn reason_code(v: ResolutionReason) -> u32 {
    match v {
        ResolutionReason::Pending => 0,
        ResolutionReason::Selected => 1,
        ResolutionReason::Timeout => 2,
    }
}
fn reason_from(v: u32) -> Result<ResolutionReason, CarrierError> {
    match v {
        0 => Ok(ResolutionReason::Pending),
        1 => Ok(ResolutionReason::Selected),
        2 => Ok(ResolutionReason::Timeout),
        _ => invalid("resolution reason invalid"),
    }
}
fn u32v(o: &mut Vec<u8>, v: u32) {
    o.extend_from_slice(&v.to_le_bytes())
}
fn u64v(o: &mut Vec<u8>, v: u64) {
    o.extend_from_slice(&v.to_le_bytes())
}
fn string(o: &mut Vec<u8>, v: &str) -> Result<(), CarrierError> {
    if v.len() > MAX_STRING {
        return invalid("string exceeds bound");
    }
    u32v(o, v.len() as u32);
    o.extend_from_slice(v.as_bytes());
    Ok(())
}
fn optional_string(o: &mut Vec<u8>, v: Option<&str>) -> Result<(), CarrierError> {
    match v {
        Some(v) => {
            u32v(o, 1);
            string(o, v)
        }
        None => {
            u32v(o, 0);
            Ok(())
        }
    }
}
fn optional_u16(o: &mut Vec<u8>, v: Option<u16>) {
    match v {
        Some(v) => {
            u32v(o, 1);
            u32v(o, u32::from(v))
        }
        None => u32v(o, 0),
    }
}
fn optional_u64(o: &mut Vec<u8>, v: Option<u64>) {
    match v {
        Some(v) => {
            u32v(o, 1);
            u64v(o, v)
        }
        None => u32v(o, 0),
    }
}
fn nonempty(v: &str, label: &str) -> Result<(), CarrierError> {
    if v.trim().is_empty() || v.len() > MAX_STRING {
        invalid(format!("{label} empty or too long"))
    } else {
        Ok(())
    }
}
fn opt_nonempty(v: Option<&str>, l: &str) -> Result<(), CarrierError> {
    v.map_or(Ok(()), |v| nonempty(v, l))
}
fn tag(v: &str) -> Result<(), CarrierError> {
    if v.len() == 3 && v.bytes().all(|b| b.is_ascii_uppercase()) && v != "---" {
        Ok(())
    } else {
        invalid("recipient tag invalid")
    }
}
fn invalid<T>(v: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(v.into()))
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CarrierError> {
        let e = self.at.checked_add(n).ok_or(CarrierError::Truncated)?;
        let v = self.data.get(self.at..e).ok_or(CarrierError::Truncated)?;
        self.at = e;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bool(&mut self) -> Result<bool, CarrierError> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("boolean invalid"),
        }
    }
    fn array32(&mut self) -> Result<[u8; 32], CarrierError> {
        Ok(self.take(32)?.try_into().unwrap())
    }
    fn string(&mut self) -> Result<String, CarrierError> {
        let n = self.u32()? as usize;
        if n > MAX_STRING {
            return invalid("string exceeds bound");
        }
        String::from_utf8(self.take(n)?.to_vec())
            .map_err(|_| CarrierError::Invalid("invalid utf8".into()))
    }
    fn optional_string(&mut self) -> Result<Option<String>, CarrierError> {
        match self.u32()? {
            0 => Ok(None),
            1 => Ok(Some(self.string()?)),
            _ => invalid("optional string marker invalid"),
        }
    }
    fn optional_u16(&mut self) -> Result<Option<u16>, CarrierError> {
        match self.u32()? {
            0 => Ok(None),
            1 => {
                Ok(Some(u16::try_from(self.u32()?).map_err(|_| {
                    CarrierError::Invalid("u16 overflow".into())
                })?))
            }
            _ => invalid("optional u16 marker invalid"),
        }
    }
    fn optional_u64(&mut self) -> Result<Option<u64>, CarrierError> {
        match self.u32()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            _ => invalid("optional u64 marker invalid"),
        }
    }
    fn telemetry(&mut self) -> Result<HookTelemetry, CarrierError> {
        Ok(HookTelemetry {
            fire_hook_validated: self.bool()?,
            option_hook_validated: self.bool()?,
            materialisation_hook_validated: self.bool()?,
            timeout_hook_validated: self.bool()?,
            all_event_classes_covered: self.bool()?,
            player_ai_timeout_no_choice_covered: self.bool()?,
            capture_start_baseline_complete: self.bool()?,
            queue_capacity: self.u32()?,
            queue_high_water_mark: self.u32()?,
            dropped_fire_rows: self.u64()?,
            dropped_resolution_rows: self.u64()?,
            correlation_failures: self.u64()?,
            invalid_definition_joins: self.u64()?,
            fired_ledger_shadow_mismatches: self.u64()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn telemetry() -> HookTelemetry {
        HookTelemetry {
            fire_hook_validated: true,
            option_hook_validated: true,
            materialisation_hook_validated: true,
            timeout_hook_validated: true,
            all_event_classes_covered: true,
            player_ai_timeout_no_choice_covered: true,
            capture_start_baseline_complete: true,
            queue_capacity: 128,
            queue_high_water_mark: 2,
            ..Default::default()
        }
    }
    fn row() -> FiredEventRow {
        FiredEventRow {
            event_id: "germany.1".into(),
            recipient_tag: "GER".into(),
            fire_date: "1936.1.1.12".into(),
            fire_hour: 0,
            fire_tick: 12,
            event_sequence: 1,
            visible_option_ordinals: vec![0, 2],
            chosen_option_ordinal: Some(2),
            selected_desc_key: Some("germany.1.d".into()),
            shown_picture_gfx: Some("GFX_report_event_german_reich".into()),
            popup_subtype: PopupSubtype::Country,
            effective_timeout_hours: 312,
            timeout_tick: 324,
            resolved_tick: Some(13),
            resolution_reason: ResolutionReason::Selected,
            runtime_title_text_context_id: None,
            runtime_description_text_context_id: None,
            runtime_option_text_contexts: Vec::new(),
        }
    }
    #[test]
    fn roundtrip_and_zero_gap() {
        let f = FiredEventFrame {
            elapsed_wall_ms: 1,
            frame_hour: 12,
            producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
            telemetry: telemetry(),
            fired_events_complete: true,
            rows: vec![row()],
        };
        let b = encode_payload(&f).unwrap();
        assert_eq!(decode_payload(&b).unwrap(), f);
        assert_eq!(encode_payload(&decode_payload(&b).unwrap()).unwrap(), b)
    }
    #[test]
    fn fail_closed_on_gap_or_bad_choice() {
        let mut t = telemetry();
        t.dropped_fire_rows = 1;
        let f = FiredEventFrame {
            elapsed_wall_ms: 1,
            frame_hour: 12,
            producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
            telemetry: t,
            fired_events_complete: true,
            rows: vec![row()],
        };
        assert!(validate_frame(&f).is_err());
        let mut r = row();
        r.chosen_option_ordinal = Some(1);
        let f = FiredEventFrame {
            elapsed_wall_ms: 1,
            frame_hour: 12,
            producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
            telemetry: telemetry(),
            fired_events_complete: true,
            rows: vec![r],
        };
        assert!(validate_frame(&f).is_err())
    }

    #[test]
    fn static_route_certification_never_relaxes_per_capture_integrity() {
        let mut failures = Vec::new();

        let mut baseline = telemetry();
        baseline.capture_start_baseline_complete = false;
        failures.push(baseline);

        let mut no_queue = telemetry();
        no_queue.queue_capacity = 0;
        failures.push(no_queue);

        let mut queue_overflow = telemetry();
        queue_overflow.queue_high_water_mark = queue_overflow.queue_capacity + 1;
        failures.push(queue_overflow);

        let mut dropped_fire = telemetry();
        dropped_fire.dropped_fire_rows = 1;
        failures.push(dropped_fire);

        let mut dropped_resolution = telemetry();
        dropped_resolution.dropped_resolution_rows = 1;
        failures.push(dropped_resolution);

        let mut correlation = telemetry();
        correlation.correlation_failures = 1;
        failures.push(correlation);

        let mut invalid_join = telemetry();
        invalid_join.invalid_definition_joins = 1;
        failures.push(invalid_join);

        let mut shadow = telemetry();
        shadow.fired_ledger_shadow_mismatches = 1;
        failures.push(shadow);

        assert!(failures.into_iter().all(|telemetry| !telemetry.zero_gap()));
    }

    #[test]
    fn ledger_preserves_repeated_same_hour_ids_by_sequence_and_no_choice_timeout() {
        let ledger = A729EventLedger::new(8);
        ledger.authenticate_installation_and_route_certification(true, true, true);
        // Hooked live dispatch can arrive before the post-install pending
        // baseline enumeration. Engine sequence, not insertion order, owns the
        // cumulative history order.
        for sequence in [11, 10] {
            ledger.record_fire(FireInput {
                event_id: "germany.1".into(),
                recipient_tag: "GER".into(),
                fire_date: "1936.1.1.12".into(),
                fire_hour: 0,
                fire_tick: 12,
                event_sequence: sequence,
                popup_subtype: PopupSubtype::News,
                effective_timeout_hours: 312,
                timeout_tick: 324,
            });
        }
        ledger.record_materialisation(MaterialisationInput {
            event_sequence: 10,
            visible_option_ordinals: vec![0, 2],
            selected_desc_key: Some("germany.1.a".into()),
            shown_picture_gfx: Some("GFX_a".into()),
            popup_subtype: PopupSubtype::Country,
            timeout_tick: 325,
            runtime_title_text_context_id: Some("event_window::title::7".into()),
            runtime_description_text_context_id: Some("event_window::description::7".into()),
            runtime_option_text_contexts: vec![OptionRuntimeTextContext {
                definition_ordinal: 0,
                context_id: "event_window::option::7".into(),
            }],
        });
        ledger.record_resolution(ResolutionInput {
            event_sequence: 10,
            chosen_option_ordinal: Some(2),
            resolved_tick: 13,
            reason: ResolutionReason::Selected,
        });
        ledger.record_materialisation(MaterialisationInput {
            event_sequence: 11,
            visible_option_ordinals: vec![],
            selected_desc_key: Some("germany.1.b".into()),
            shown_picture_gfx: Some("GFX_b".into()),
            popup_subtype: PopupSubtype::News,
            timeout_tick: 324,
            runtime_title_text_context_id: None,
            runtime_description_text_context_id: None,
            runtime_option_text_contexts: Vec::new(),
        });
        ledger.record_resolution(ResolutionInput {
            event_sequence: 11,
            chosen_option_ordinal: None,
            resolved_tick: 324,
            reason: ResolutionReason::Timeout,
        });
        let encoded = collect_and_encode(&ledger, 0, 1).unwrap();
        let decoded = decode_payload(&encoded).unwrap();
        assert!(decoded.fired_events_complete);
        assert_eq!(decoded.rows.len(), 2);
        assert_eq!(decoded.rows[0].event_sequence, 10);
        assert_eq!(decoded.rows[0].effective_timeout_hours, 313);
        assert_eq!(decoded.rows[0].timeout_tick, 325);
        assert_eq!(
            decoded.rows[0]
                .runtime_description_text_context_id
                .as_deref(),
            Some("event_window::description::7")
        );
        assert_eq!(
            decoded.rows[0].runtime_option_text_contexts,
            vec![OptionRuntimeTextContext {
                definition_ordinal: 0,
                context_id: "event_window::option::7".into(),
            }]
        );
        assert_eq!(decoded.rows[1].event_sequence, 11);
        assert!(decoded.rows[1].visible_option_ordinals.is_empty());
    }

    #[test]
    fn ai_selected_ordinal_is_valid_without_popup_materialisation() {
        let mut ai = row();
        ai.visible_option_ordinals.clear();
        ai.selected_desc_key = None;
        ai.shown_picture_gfx = None;
        ai.popup_subtype = PopupSubtype::News;
        ai.chosen_option_ordinal = Some(2);
        let frame = FiredEventFrame {
            elapsed_wall_ms: 1,
            frame_hour: 12,
            producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
            telemetry: telemetry(),
            fired_events_complete: true,
            rows: vec![ai],
        };
        assert!(validate_frame(&frame).is_ok());
    }

    #[test]
    fn timeout_default_ordinal_and_timeout_no_choice_remain_distinct() {
        let mut timeout_default = row();
        timeout_default.resolution_reason = ResolutionReason::Timeout;
        timeout_default.resolved_tick = Some(timeout_default.timeout_tick);
        timeout_default.chosen_option_ordinal = Some(0);

        let mut timeout_no_choice = timeout_default.clone();
        timeout_no_choice.event_sequence = 2;
        timeout_no_choice.fire_tick = 13;
        timeout_no_choice.timeout_tick = 325;
        timeout_no_choice.resolved_tick = Some(325);
        timeout_no_choice.visible_option_ordinals.clear();
        timeout_no_choice.chosen_option_ordinal = None;
        timeout_no_choice.selected_desc_key = None;
        timeout_no_choice.shown_picture_gfx = None;

        let frame = FiredEventFrame {
            elapsed_wall_ms: 1,
            frame_hour: 12,
            producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
            telemetry: telemetry(),
            fired_events_complete: true,
            rows: vec![timeout_default, timeout_no_choice],
        };
        let decoded = decode_payload(&encode_payload(&frame).unwrap()).unwrap();
        assert_eq!(decoded.rows[0].chosen_option_ordinal, Some(0));
        assert_eq!(decoded.rows[1].chosen_option_ordinal, None);
    }

    #[test]
    fn runtime_option_contexts_require_unique_visible_definition_ordinals() {
        let mut invalid = row();
        invalid.runtime_option_text_contexts = vec![OptionRuntimeTextContext {
            definition_ordinal: 1,
            context_id: "EventWindow::Name::1".into(),
        }];
        let mut frame = FiredEventFrame {
            elapsed_wall_ms: 1,
            frame_hour: 12,
            producer_manifest_sha256: PRODUCER_MANIFEST_SHA256,
            telemetry: telemetry(),
            fired_events_complete: true,
            rows: vec![invalid],
        };
        assert!(validate_frame(&frame).is_err());
        frame.rows[0].runtime_option_text_contexts = vec![
            OptionRuntimeTextContext {
                definition_ordinal: 2,
                context_id: "EventWindow::Name::2".into(),
            },
            OptionRuntimeTextContext {
                definition_ordinal: 2,
                context_id: "EventWindow::Name::3".into(),
            },
        ];
        assert!(validate_frame(&frame).is_err());
    }
}
