//! Dormant, evidence-only automation for the A25 owner-scoped render probe.
//!
//! Exact detours call this state machine while raw pointers are live.  It
//! buffers by the concrete render candidate and publishes nothing until the
//! production selected-Present boundary chooses that candidate.  The module
//! has a lane-owned evidence activation switch which is mechanically separate
//! from production promotion: the owner-render, bitmap-font,
//! glyph-preparation, and inline-icon detours remain disabled until the
//! retained H-A25 live fixture has been accepted.

use crate::a729_hook::{
    install_admitted_owner_capture, observed_owner_for_render_layout,
    ObservedOwnerRenderJoin,
};
use crate::owner_glyph_capture::{
    AdmittedOwnerGlyphCapture, CaptureHealth, FontAssetReceipt, GlyphDescriptorIdentity,
    InlineIconAssetReceipt, OwnerGlyphCaptureAdmission, OwnerGlyphCaptureAuthority,
    OwnerGlyphRowIdentity, OwnerScopedGlyphCapture, OwnerScopedGlyphObservation,
    OwnerScopedInlineIconObservation,
};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Lane-owned authorization for the raw harness probe. This is deliberately
/// not the d76 production-promotion latch.
pub const OWNER_GLYPH_CAPTURE_ACTIVATION: bool = false;
pub const A25_OWNER_GLYPH_EVIDENCE_MODE_ENABLED: bool = OWNER_GLYPH_CAPTURE_ACTIVATION;
pub const MAX_COMPLETED_OWNER_GLYPH_CANDIDATES: usize = 16_384;

static EVIDENCE_AUTOMATION: OnceLock<Mutex<Option<OwnerGlyphEvidenceAutomation>>> =
    OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerGlyphPresentCandidate {
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub submit_index: u64,
    pub frame_sequence: u64,
    /// Canonical compact hour index belonging to the candidate Present.
    pub frame_hour: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedOwnerGlyphCapture {
    pub candidate: OwnerGlyphPresentCandidate,
    pub context_id: String,
    pub view_id: String,
    pub element_id: String,
    pub admitted: AdmittedOwnerGlyphCapture,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnerGlyphProbeTelemetry {
    pub completed_count: u64,
    pub discarded_non_selected_count: u64,
    pub rejected_count: u64,
    pub reentrancy_count: u64,
    pub queue_overflow_count: u64,
}

struct ActiveInvocation {
    owner: usize,
    owner_join: ObservedOwnerRenderJoin,
    candidate: OwnerGlyphPresentCandidate,
    frame_hour: u64,
    render_invocation_ordinal: u64,
    clip: Option<[i32; 4]>,
    glyphs: Vec<OwnerScopedGlyphObservation>,
    inline_icons: Vec<OwnerScopedInlineIconObservation>,
    fonts: Vec<FontAssetReceipt>,
    icon_assets: Vec<InlineIconAssetReceipt>,
    health: CaptureHealth,
}

struct CompletedInvocation {
    owner: usize,
    candidate: OwnerGlyphPresentCandidate,
    row_identity: OwnerGlyphRowIdentity,
    capture: OwnerScopedGlyphCapture,
}

/// One hook-thread state machine.  The owner-render detour owns this value;
/// glyph/font/icon callbacks on any other thread cannot accidentally join it.
pub struct OwnerGlyphEvidenceAutomation {
    process_identity: String,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    next_render_invocation_ordinal: u64,
    active: Option<ActiveInvocation>,
    completed: VecDeque<CompletedInvocation>,
    admission: OwnerGlyphCaptureAdmission,
    telemetry: OwnerGlyphProbeTelemetry,
}

impl OwnerGlyphEvidenceAutomation {
    pub fn dormant(
        process_identity: String,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
    ) -> Result<Self, String> {
        if process_identity.trim().is_empty()
            || process_start_nonce == 0
            || hook_epoch_nonce == 0
        {
            return Err("A25 owner-glyph evidence epoch is incomplete".into());
        }
        Ok(Self {
            process_identity,
            process_start_nonce,
            hook_epoch_nonce,
            next_render_invocation_ordinal: 0,
            active: None,
            completed: VecDeque::new(),
            admission: OwnerGlyphCaptureAdmission::default(),
            telemetry: OwnerGlyphProbeTelemetry::default(),
        })
    }

    pub fn begin_owner_render(
        &mut self,
        layout_object: usize,
        candidate: OwnerGlyphPresentCandidate,
        frame_hour: u64,
        clip: Option<[i32; 4]>,
    ) -> Result<(), String> {
        self.validate_candidate(candidate, frame_hour)?;
        if self.active.is_some() {
            self.telemetry.reentrancy_count = self.telemetry.reentrancy_count.saturating_add(1);
            self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
            if let Some(active) = self.active.as_mut() {
                active.health.reentrancy_count = active.health.reentrancy_count.saturating_add(1);
            }
            return Err("A25 owner-glyph render invocation is reentrant".into());
        }
        let owner_join = observed_owner_for_render_layout(layout_object)
            .ok_or("A25 render layout does not resolve to exactly one registered textbox")?;
        let ordinal = self.next_render_invocation_ordinal;
        self.next_render_invocation_ordinal = self
            .next_render_invocation_ordinal
            .checked_add(1)
            .ok_or("A25 render invocation ordinal overflowed")?;
        self.active = Some(ActiveInvocation {
            owner: owner_join.owner,
            owner_join,
            candidate,
            frame_hour,
            render_invocation_ordinal: ordinal,
            clip,
            glyphs: Vec::new(),
            inline_icons: Vec::new(),
            fonts: Vec::new(),
            icon_assets: Vec::new(),
            health: CaptureHealth::default(),
        });
        Ok(())
    }

    pub fn record_font_asset(&mut self, receipt: FontAssetReceipt) -> Result<(), String> {
        let active = self.active_mut()?;
        active.fonts.push(receipt);
        Ok(())
    }

    pub fn record_inline_icon_asset(
        &mut self,
        receipt: InlineIconAssetReceipt,
    ) -> Result<(), String> {
        let active = self.active_mut()?;
        active.icon_assets.push(receipt);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_ordinary_glyph(
        &mut self,
        font_identity: String,
        descriptor: GlyphDescriptorIdentity,
        glyph_id: u32,
        owner_relative_x: i32,
        owner_relative_y: i32,
        rgba: [u8; 4],
        generated_vertices: Vec<u8>,
    ) -> Result<(), String> {
        let active = self.active_mut()?;
        let run_ordinal = u32::try_from(active.glyphs.len() + active.inline_icons.len())
            .map_err(|_| "A25 glyph/icon run count exceeds u32".to_string())?;
        active.glyphs.push(OwnerScopedGlyphObservation {
            run_ordinal,
            font_identity,
            descriptor,
            glyph_id,
            owner_relative_x,
            owner_relative_y,
            rgba,
            generated_vertices,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_inline_icon(
        &mut self,
        token: String,
        sprite_identity: String,
        frame: u32,
        owner_relative_x: i32,
        owner_relative_y: i32,
        baseline: i32,
        advance: i32,
        draw_receipt_sha256: String,
    ) -> Result<(), String> {
        let active = self.active_mut()?;
        let run_ordinal = u32::try_from(active.glyphs.len() + active.inline_icons.len())
            .map_err(|_| "A25 glyph/icon run count exceeds u32".to_string())?;
        active.inline_icons.push(OwnerScopedInlineIconObservation {
            run_ordinal,
            token,
            sprite_identity,
            frame,
            owner_relative_x,
            owner_relative_y,
            baseline,
            advance,
            draw_receipt_sha256,
        });
        Ok(())
    }

    pub fn note_parse_failure(&mut self) {
        self.note_active_failure(|health| {
            health.parse_failure_count = health.parse_failure_count.saturating_add(1)
        });
    }

    pub fn note_gap(&mut self) {
        self.note_active_failure(|health| {
            health.gap_count = health.gap_count.saturating_add(1)
        });
    }

    pub fn note_drop(&mut self) {
        self.note_active_failure(|health| {
            health.drop_count = health.drop_count.saturating_add(1)
        });
    }

    pub fn note_hook_failure(&mut self) {
        self.note_active_failure(|health| {
            health.hook_failure_count = health.hook_failure_count.saturating_add(1)
        });
    }

    pub fn finish_owner_render(&mut self) -> Result<(), String> {
        let active = self
            .active
            .take()
            .ok_or("A25 owner-glyph render exit has no active invocation")?;
        if self.completed.len() == MAX_COMPLETED_OWNER_GLYPH_CANDIDATES {
            self.telemetry.queue_overflow_count =
                self.telemetry.queue_overflow_count.saturating_add(1);
            self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
            return Err("A25 owner-glyph candidate queue reached its fail-closed bound".into());
        }
        let glyph_decision_count = u32::try_from(active.glyphs.len())
            .map_err(|_| "A25 glyph decision count exceeds u32".to_string())?;
        let inline_icon_decision_count = u32::try_from(active.inline_icons.len())
            .map_err(|_| "A25 inline-icon decision count exceeds u32".to_string())?;
        let capture = OwnerScopedGlyphCapture {
            authority: OwnerGlyphCaptureAuthority::exact_a729(),
            process_identity: self.process_identity.clone(),
            hook_epoch: self.hook_epoch_nonce,
            frame_hour: active.frame_hour,
            render_invocation_ordinal: active.render_invocation_ordinal,
            context_id: active.owner_join.context_id,
            view_id: active.owner_join.view_id,
            element_id: active.owner_join.element_id,
            source: active.owner_join.source,
            selected_final_text_sha256: active.owner_join.selected_final_text_sha256,
            clip: active.clip,
            glyph_decision_count,
            inline_icon_decision_count,
            glyphs: active.glyphs,
            inline_icons: active.inline_icons,
            fonts: active.fonts,
            icon_assets: active.icon_assets,
            health: active.health,
        };
        let row_identity = OwnerGlyphRowIdentity {
            context_id: capture.context_id.clone(),
            view_id: capture.view_id.clone(),
            element_id: capture.element_id.clone(),
            source: capture.source.clone(),
            selected_final_text_bytes: active.owner_join.selected_final_text_bytes,
            selected_final_text_sha256: capture.selected_final_text_sha256.clone(),
        };
        self.completed.push_back(CompletedInvocation {
            owner: active.owner,
            candidate: active.candidate,
            row_identity,
            capture,
        });
        self.telemetry.completed_count = self.telemetry.completed_count.saturating_add(1);
        Ok(())
    }

    /// Select and admit only observations from the exact chosen Present.
    /// Earlier same-hour candidates are destroyed here; timing proximity is
    /// never used to move them to the selected frame.
    pub fn admit_selected_present(
        &mut self,
        selected: OwnerGlyphPresentCandidate,
        frame_hour: u64,
    ) -> Result<Vec<SelectedOwnerGlyphCapture>, String> {
        self.validate_candidate(selected, frame_hour)?;
        if self.active.is_some() {
            return Err("A25 selected-Present drain crossed an active owner render".into());
        }
        let mut admitted = Vec::new();
        let mut future = VecDeque::new();
        while let Some(completed) = self.completed.pop_front() {
            if completed.candidate.frame_hour > selected.frame_hour {
                future.push_back(completed);
                continue;
            }
            let selected_capture = completed.candidate == selected;
            let admitted_capture = self.admission.admit_against_identity(
                completed.capture,
                &completed.row_identity,
                completed.candidate.frame_hour,
            )?;
            if !selected_capture {
                self.telemetry.discarded_non_selected_count = self
                    .telemetry
                    .discarded_non_selected_count
                    .saturating_add(1);
                continue;
            }
            install_admitted_owner_capture(
                completed.owner,
                &completed.row_identity.context_id,
                &completed.row_identity.selected_final_text_sha256,
                &admitted_capture,
            )?;
            admitted.push(SelectedOwnerGlyphCapture {
                candidate: selected,
                context_id: completed.row_identity.context_id,
                view_id: completed.row_identity.view_id,
                element_id: completed.row_identity.element_id,
                admitted: admitted_capture,
            });
        }
        self.completed = future;
        Ok(admitted)
    }

    pub fn telemetry(&self) -> OwnerGlyphProbeTelemetry {
        self.telemetry
    }

    fn active_mut(&mut self) -> Result<&mut ActiveInvocation, String> {
        if self.active.is_none() {
            self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
        }
        self.active
            .as_mut()
            .ok_or_else(|| "A25 glyph/icon callback is outside an owner render".to_string())
    }

    fn note_active_failure(&mut self, note: impl FnOnce(&mut CaptureHealth)) {
        if let Some(active) = self.active.as_mut() {
            note(&mut active.health);
        } else {
            self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
        }
    }

    fn validate_candidate(
        &self,
        candidate: OwnerGlyphPresentCandidate,
        frame_hour: u64,
    ) -> Result<(), String> {
        if candidate.process_start_nonce != self.process_start_nonce
            || candidate.hook_epoch_nonce != self.hook_epoch_nonce
            || candidate.submit_index == 0
            || candidate.frame_sequence == 0
            || candidate.frame_hour != frame_hour
        {
            return Err("A25 owner-glyph candidate epoch/hour identity is invalid".into());
        }
        Ok(())
    }
}

fn automation_slot() -> &'static Mutex<Option<OwnerGlyphEvidenceAutomation>> {
    EVIDENCE_AUTOMATION.get_or_init(|| Mutex::new(None))
}

/// Explicit harness-only activation. Production scheduling does not call this
/// while `A25_OWNER_GLYPH_EVIDENCE_MODE_ENABLED` is false.
pub fn install_dormant_evidence_automation(
    process_identity: String,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
) -> Result<(), String> {
    if !OWNER_GLYPH_CAPTURE_ACTIVATION {
        return Err(
            "A25 owner-glyph evidence automation is not authorized by the lane-owned evidence latch"
                .into(),
        );
    }
    let automation = OwnerGlyphEvidenceAutomation::dormant(
        process_identity,
        process_start_nonce,
        hook_epoch_nonce,
    )?;
    let mut slot = automation_slot()
        .lock()
        .map_err(|_| "A25 owner-glyph evidence automation state poisoned".to_string())?;
    if slot.is_some() {
        return Err("A25 owner-glyph evidence automation is already installed".into());
    }
    *slot = Some(automation);
    Ok(())
}

#[cfg(test)]
mod activation_tests {
    use super::*;

    #[test]
    fn dormant_evidence_install_cannot_bypass_false_lane_latch() {
        assert!(!A25_OWNER_GLYPH_EVIDENCE_MODE_ENABLED);
        assert!(!OWNER_GLYPH_CAPTURE_ACTIVATION);
        let error = install_dormant_evidence_automation(
            "exact-a729-fixture".into(),
            1,
            1,
        )
        .expect_err("false evidence latch must reject before installing state");
        assert!(error.contains("lane-owned evidence latch"));
    }
}

pub fn uninstall_dormant_evidence_automation() -> Result<OwnerGlyphProbeTelemetry, String> {
    let mut slot = automation_slot()
        .lock()
        .map_err(|_| "A25 owner-glyph evidence automation state poisoned".to_string())?;
    if slot
        .as_ref()
        .ok_or("A25 owner-glyph evidence automation is not installed")?
        .active
        .is_some()
    {
        return Err("A25 owner-glyph automation teardown crossed an active render".into());
    }
    let automation = slot.take().expect("automation presence checked above");
    Ok(automation.telemetry())
}

pub fn with_dormant_evidence_automation<T>(
    operation: impl FnOnce(&mut OwnerGlyphEvidenceAutomation) -> Result<T, String>,
) -> Result<Option<T>, String> {
    let mut slot = automation_slot()
        .lock()
        .map_err(|_| "A25 owner-glyph evidence automation state poisoned".to_string())?;
    let Some(automation) = slot.as_mut() else {
        return Ok(None);
    };
    operation(automation).map(Some)
}

pub fn admit_selected_present_if_evidence_active(
    selected: OwnerGlyphPresentCandidate,
) -> Result<Vec<SelectedOwnerGlyphCapture>, String> {
    with_dormant_evidence_automation(|automation| {
        automation.admit_selected_present(selected, selected.frame_hour)
    })
    .map(|selected| selected.unwrap_or_default())
}
