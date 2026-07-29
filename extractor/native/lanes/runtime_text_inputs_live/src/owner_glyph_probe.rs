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
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, ThreadId};

/// Lane-owned authorization for the raw harness probe.
pub const OWNER_GLYPH_CAPTURE_ACTIVATION: bool = false;
pub const A25_OWNER_GLYPH_EVIDENCE_MODE_ENABLED: bool = OWNER_GLYPH_CAPTURE_ACTIVATION;
pub const MAX_COMPLETED_OWNER_GLYPH_CANDIDATES: usize = 16_384;
pub const GLYPH_BUFFER_CACHE_SIDECAR_SLOT_COUNT: usize =
    crate::owner_glyph_capture::GLYPH_BUFFER_CACHE_ENTRY_COUNT as usize;

static OWNER_GLYPH_AUTOMATION: OnceLock<Mutex<Option<OwnerGlyphEvidenceAutomation>>> =
    OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutomationMode {
    Evidence,
    Production,
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedOwnerGlyphCaptureReservation {
    pub reservation_id: u64,
    pub captures: Vec<SelectedOwnerGlyphCapture>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnerGlyphProbeTelemetry {
    pub completed_count: u64,
    pub discarded_non_selected_count: u64,
    pub rejected_count: u64,
    pub reentrancy_count: u64,
    pub queue_overflow_count: u64,
}

/// Pointer-free payload produced at exact-a729 `0x14228C585`, after the six
/// ordinary-glyph vertices have been written but before owner identity is lost
/// to grouping. The native hook must copy descriptor/vertex bytes while the
/// invocation is live; no raw address may cross this API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostEmitOrdinaryGlyphObservation {
    pub font_identity: String,
    pub requested_codepoint: u32,
    pub resolved_codepoint: u32,
    pub descriptor_sha256: String,
    pub source_ordinal: u32,
    pub emitted_vertex_end_count: u32,
    /// RVA of the first copied byte. The native seam computes this from the
    /// live scratch base and post-write count before copying the 168 bytes.
    pub scratch_group_start_rva: u32,
    pub generated_vertices: Vec<u8>,
}

/// Process-local identity of one occupied exact-a729 bitmap-font cache entry.
///
/// `font_instance_token` is the exact `CBitmapFont*` bit pattern and
/// `buffer_identity` is the exact GPU-buffer handle stored at entry `+0x18`.
/// They are deliberately confined to this live sidecar and are never emitted
/// into a replay artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphBufferCacheIdentity {
    pub font_instance_token: u64,
    pub font_identity: String,
    pub slot_index: u8,
    pub cache_key: i32,
    pub texture_or_page_id: i32,
    pub vertex_count: u32,
    pub buffer_identity: u64,
}

impl GlyphBufferCacheIdentity {
    fn validate(&self) -> Result<(), String> {
        if self.font_instance_token == 0 {
            return Err("A25 glyph-cache identity has a null font instance".into());
        }
        if self.font_identity.trim().is_empty() {
            return Err("A25 glyph-cache identity has no physical font identity".into());
        }
        if usize::from(self.slot_index) >= GLYPH_BUFFER_CACHE_SIDECAR_SLOT_COUNT {
            return Err("A25 glyph-cache slot exceeds the exact 40-entry bound".into());
        }
        if self.cache_key == 0 {
            return Err("A25 glyph-cache identity uses the engine empty-slot key".into());
        }
        if self.vertex_count == 0
            || self.vertex_count > 3072
            || self.vertex_count % crate::owner_glyph_capture::GLYPH_VERTEX_COUNT as u32 != 0
        {
            return Err("A25 glyph-cache identity has an invalid exact vertex count".into());
        }
        if self.buffer_identity == 0 {
            return Err("A25 glyph-cache identity has a null GPU buffer".into());
        }
        Ok(())
    }

    fn at_slot(&self, slot_index: u8) -> Self {
        let mut moved = self.clone();
        moved.slot_index = slot_index;
        moved
    }
}

#[derive(Clone)]
struct SealedGlyphBufferCacheEntry {
    identity: GlyphBufferCacheIdentity,
    selected_final_text_sha256: String,
    glyphs: Vec<OwnerScopedGlyphObservation>,
    font_asset: FontAssetReceipt,
}

struct FontGlyphBufferCacheSidecar {
    font_identity: String,
    slots: Vec<Option<SealedGlyphBufferCacheEntry>>,
}

impl FontGlyphBufferCacheSidecar {
    fn new(font_identity: String) -> Self {
        Self {
            font_identity,
            slots: (0..GLYPH_BUFFER_CACHE_SIDECAR_SLOT_COUNT)
                .map(|_| None)
                .collect(),
        }
    }
}

/// Exact process-local mirror of each observed font object's 40 engine slots.
/// A font is retained only until its destructor callback or automation
/// teardown. Each font cache is therefore structurally bounded to the engine's
/// exact 40 entries; no glyph payload is inferred for an unobserved slot.
#[derive(Default)]
struct GlyphBufferCacheSidecars {
    fonts: BTreeMap<u64, FontGlyphBufferCacheSidecar>,
}

impl GlyphBufferCacheSidecars {
    fn seal_population(
        &mut self,
        identity: GlyphBufferCacheIdentity,
        selected_final_text_sha256: String,
        glyphs: Vec<OwnerScopedGlyphObservation>,
        font_assets: &[FontAssetReceipt],
    ) -> Result<(), String> {
        identity.validate()?;
        require_lower_hex_sha256(
            &selected_final_text_sha256,
            "A25 glyph-cache selected final text",
        )?;
        if glyphs.is_empty() {
            return Err("A25 glyph-cache population has no ordinary glyph runs".into());
        }
        if glyphs.iter().enumerate().any(|(ordinal, glyph)| {
            glyph.run_ordinal as usize != ordinal || glyph.font_identity != identity.font_identity
        }) {
            return Err(
                "A25 glyph-cache population has reordered runs or a mismatched font".into(),
            );
        }
        let observed_vertex_count = glyphs
            .len()
            .checked_mul(crate::owner_glyph_capture::GLYPH_VERTEX_COUNT)
            .and_then(|count| u32::try_from(count).ok())
            .ok_or("A25 glyph-cache population vertex count overflowed")?;
        if identity.vertex_count != observed_vertex_count {
            return Err(
                "A25 glyph-cache population vertex count disagrees with sealed glyph runs".into(),
            );
        }
        let mut matching_assets = font_assets
            .iter()
            .filter(|receipt| receipt.identity == identity.font_identity);
        let font_asset = matching_assets
            .next()
            .cloned()
            .ok_or("A25 glyph-cache population has no authenticated font asset")?;
        if matching_assets.next().is_some() {
            return Err("A25 glyph-cache population has duplicate font assets".into());
        }

        let font = self
            .fonts
            .entry(identity.font_instance_token)
            .or_insert_with(|| FontGlyphBufferCacheSidecar::new(identity.font_identity.clone()));
        if font.font_identity != identity.font_identity {
            return Err(
                "A25 glyph-cache font-instance token was reused without teardown".into(),
            );
        }
        let slot = usize::from(identity.slot_index);
        if font.slots[slot].is_some() {
            return Err(
                "A25 glyph-cache population overwrote a slot without exact zero invalidation"
                    .into(),
            );
        }

        // Stale selection zeroes entry +0 before overwrite. The zero callback
        // must remove the old sidecar first; population can never silently
        // replace either the same tuple or a different one.
        font.slots[slot] = Some(SealedGlyphBufferCacheEntry {
            identity,
            selected_final_text_sha256,
            glyphs,
            font_asset,
        });
        Ok(())
    }

    /// Mirror one observed engine hit before deciding whether its sidecar can
    /// be admitted to the current owner. Exact-a729 swaps every hit with slot
    /// zero even when capture cannot replay it, so ordering is state evidence,
    /// not a consequence of replay admission.
    fn observe_hit_reorder(
        &mut self,
        identity: &GlyphBufferCacheIdentity,
    ) -> Result<Option<SealedGlyphBufferCacheEntry>, String> {
        identity.validate()?;
        let Some(font) = self.fonts.get(&identity.font_instance_token) else {
            return Ok(None);
        };
        if font.font_identity != identity.font_identity {
            self.fonts.remove(&identity.font_instance_token);
            return Err("A25 glyph-cache hit font identity mismatch".into());
        }
        let hit_slot = usize::from(identity.slot_index);
        let mismatch = font.slots[hit_slot]
            .as_ref()
            .is_some_and(|sealed| &sealed.identity != identity);
        if mismatch {
            // A tracked but different tuple means an eviction/reorder callback
            // was missed. The entire font mirror is no longer trustworthy.
            self.fonts.remove(&identity.font_instance_token);
            return Err("A25 glyph-cache hit slot/key/buffer identity mismatch".into());
        }
        let same_tuple_elsewhere = font.slots[hit_slot].is_none()
            && font.slots.iter().enumerate().any(|(slot, sealed)| {
                slot != hit_slot
                    && sealed.as_ref().is_some_and(|sealed| {
                        sealed.identity.font_instance_token == identity.font_instance_token
                            && sealed.identity.font_identity == identity.font_identity
                            && sealed.identity.cache_key == identity.cache_key
                            && sealed.identity.texture_or_page_id == identity.texture_or_page_id
                            && sealed.identity.vertex_count == identity.vertex_count
                            && sealed.identity.buffer_identity == identity.buffer_identity
                    })
            });
        if same_tuple_elsewhere {
            // The exact tuple was already mirrored into another slot by a
            // prior hit. Seeing it again at the stale pre-hit slot proves the
            // callback chronology no longer matches the engine cache. Do not
            // swap the trusted entry back into the stale slot.
            self.fonts.remove(&identity.font_instance_token);
            return Err("A25 glyph-cache hit repeated a stale pre-hit slot".into());
        }

        let font = self
            .fonts
            .get_mut(&identity.font_instance_token)
            .ok_or("A25 glyph-cache hit font sidecar disappeared during reorder")?;
        let payload = font.slots[hit_slot].clone();
        if hit_slot != 0 {
            // `None` represents an engine entry that predates observation. It
            // must still displace any tracked slot-zero sidecar to the hit
            // slot, leaving the new unseen slot zero unavailable.
            font.slots.swap(0, hit_slot);
            if let Some(moved_hit) = font.slots[0].as_mut() {
                moved_hit.identity.slot_index = 0;
            }
            if let Some(displaced) = font.slots[hit_slot].as_mut() {
                displaced.identity.slot_index = identity.slot_index;
            }
        }
        Ok(payload)
    }

    fn evict_exact(&mut self, identity: &GlyphBufferCacheIdentity) -> Result<(), String> {
        identity.validate()?;
        let Some(font) = self.fonts.get_mut(&identity.font_instance_token) else {
            // Zeroing an entry that predates attachment has no sidecar to
            // invalidate and cannot make a later hit admissible.
            return Ok(());
        };
        if font.font_identity != identity.font_identity {
            return Err("A25 glyph-cache eviction font identity mismatch".into());
        }
        let slot = &mut font.slots[usize::from(identity.slot_index)];
        let Some(sealed) = slot.as_ref() else {
            return Ok(());
        };
        if &sealed.identity != identity {
            return Err("A25 glyph-cache eviction slot/key/buffer identity mismatch".into());
        }
        *slot = None;
        Ok(())
    }

    fn clear_font(
        &mut self,
        font_instance_token: u64,
        font_identity: &str,
    ) -> Result<(), String> {
        if font_instance_token == 0 || font_identity.trim().is_empty() {
            return Err("A25 glyph-cache font teardown identity is incomplete".into());
        }
        let Some(font) = self.fonts.get(&font_instance_token) else {
            return Ok(());
        };
        if font.font_identity != font_identity {
            return Err("A25 glyph-cache font teardown identity mismatch".into());
        }
        self.fonts.remove(&font_instance_token);
        Ok(())
    }

    fn clear(&mut self) {
        self.fonts.clear();
    }
}

fn require_lower_hex_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} SHA-256 is not lowercase hex"));
    }
    Ok(())
}

fn rebase_cached_glyphs(
    mut glyphs: Vec<OwnerScopedGlyphObservation>,
    destination_rect: [i32; 4],
) -> Result<Vec<OwnerScopedGlyphObservation>, String> {
    for glyph in &mut glyphs {
        if glyph.generated_vertices.len() != crate::owner_glyph_capture::GLYPH_VERTEX_BYTES {
            return Err("A25 cached glyph has an invalid vertex receipt length".into());
        }
        let mut min_x = None::<f32>;
        let mut min_y = None::<f32>;
        for vertex in glyph
            .generated_vertices
            .chunks_exact(crate::owner_glyph_capture::GLYPH_VERTEX_STRIDE)
        {
            let coordinate = |offset: usize| {
                f32::from_le_bytes([
                    vertex[offset],
                    vertex[offset + 1],
                    vertex[offset + 2],
                    vertex[offset + 3],
                ])
            };
            let x = coordinate(0);
            let y = coordinate(4);
            if !x.is_finite() || !y.is_finite() {
                return Err("A25 cached glyph contains non-finite absolute geometry".into());
            }
            min_x = Some(min_x.map_or(x, |value| value.min(x)));
            min_y = Some(min_y.map_or(y, |value| value.min(y)));
        }
        let exact_i32 = |value: f32, label: &str| -> Result<i32, String> {
            if value < i32::MIN as f32 || value > i32::MAX as f32 || value.fract() != 0.0 {
                return Err(format!("A25 cached glyph absolute {label} is not an exact integer"));
            }
            Ok(value as i32)
        };
        let absolute_x = exact_i32(min_x.ok_or("A25 cached glyph has no x")?, "x")?;
        let absolute_y = exact_i32(min_y.ok_or("A25 cached glyph has no y")?, "y")?;
        glyph.owner_relative_x = absolute_x
            .checked_sub(destination_rect[0])
            .ok_or("A25 cached glyph owner-relative x overflow")?;
        glyph.owner_relative_y = absolute_y
            .checked_sub(destination_rect[1])
            .ok_or("A25 cached glyph owner-relative y overflow")?;
    }
    Ok(glyphs)
}

struct ActiveInvocation {
    owner_thread_id: ThreadId,
    owner: usize,
    owner_join: ObservedOwnerRenderJoin,
    candidate: OwnerGlyphPresentCandidate,
    frame_hour: u64,
    render_invocation_ordinal: u64,
    gui_engine_instance_token: u64,
    paint_ordinal: u32,
    destination_rect: [i32; 4],
    ui_clip: Option<[i32; 4]>,
    clip: Option<[i32; 4]>,
    last_glyph_source_ordinal: Option<u32>,
    last_emitted_vertex_end_count: Option<u32>,
    cache_hit_replayed: bool,
    cache_population_sealed: bool,
    glyphs: Vec<OwnerScopedGlyphObservation>,
    inline_icons: Vec<OwnerScopedInlineIconObservation>,
    fonts: Vec<FontAssetReceipt>,
    icon_assets: Vec<InlineIconAssetReceipt>,
    health: CaptureHealth,
}

#[derive(Clone)]
struct CompletedInvocation {
    owner: usize,
    candidate: OwnerGlyphPresentCandidate,
    row_identity: OwnerGlyphRowIdentity,
    capture: OwnerScopedGlyphCapture,
}

#[derive(Clone)]
struct SelectedOwnerInstall {
    owner: usize,
    context_id: String,
    selected_final_text_sha256: String,
    admitted: AdmittedOwnerGlyphCapture,
}

#[derive(Clone)]
struct OwnerGlyphSelectionReservation {
    reservation_id: u64,
    selected: OwnerGlyphPresentCandidate,
    captures: Vec<SelectedOwnerGlyphCapture>,
    installs: Vec<SelectedOwnerInstall>,
    admission_after: OwnerGlyphCaptureAdmission,
    discarded_non_selected_count: u64,
    active: bool,
}

/// One hook-thread state machine.  The owner-render detour owns this value;
/// glyph/font/icon callbacks on any other thread cannot accidentally join it.
pub struct OwnerGlyphEvidenceAutomation {
    mode: AutomationMode,
    process_identity: String,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    next_render_invocation_ordinal: u64,
    active: Option<ActiveInvocation>,
    completed: VecDeque<CompletedInvocation>,
    admission: OwnerGlyphCaptureAdmission,
    next_selection_reservation_id: u64,
    selection_reservation: Option<OwnerGlyphSelectionReservation>,
    glyph_buffer_cache_sidecars: GlyphBufferCacheSidecars,
    telemetry: OwnerGlyphProbeTelemetry,
}

impl OwnerGlyphEvidenceAutomation {
    pub fn dormant(
        process_identity: String,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
    ) -> Result<Self, String> {
        Self::new(
            AutomationMode::Evidence,
            process_identity,
            process_start_nonce,
            hook_epoch_nonce,
        )
    }

    fn production(
        process_identity: String,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
    ) -> Result<Self, String> {
        Self::new(
            AutomationMode::Production,
            process_identity,
            process_start_nonce,
            hook_epoch_nonce,
        )
    }

    fn new(
        mode: AutomationMode,
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
            mode,
            process_identity,
            process_start_nonce,
            hook_epoch_nonce,
            next_render_invocation_ordinal: 0,
            active: None,
            completed: VecDeque::new(),
            admission: OwnerGlyphCaptureAdmission::default(),
            next_selection_reservation_id: 1,
            selection_reservation: None,
            glyph_buffer_cache_sidecars: GlyphBufferCacheSidecars::default(),
            telemetry: OwnerGlyphProbeTelemetry::default(),
        })
    }

    pub fn begin_owner_render(
        &mut self,
        layout_object: usize,
        candidate: OwnerGlyphPresentCandidate,
        frame_hour: u64,
        gui_engine_instance_token: u64,
        paint_ordinal: u32,
        destination_rect: [i32; 4],
        ui_clip: Option<[i32; 4]>,
        clip: Option<[i32; 4]>,
    ) -> Result<(), String> {
        self.validate_candidate(candidate, frame_hour)?;
        if gui_engine_instance_token == 0 {
            return Err("A25 owner render has no common GUI engine-instance token".into());
        }
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
            owner_thread_id: thread::current().id(),
            owner: owner_join.owner,
            owner_join,
            candidate,
            frame_hour,
            render_invocation_ordinal: ordinal,
            gui_engine_instance_token,
            paint_ordinal,
            destination_rect,
            ui_clip,
            clip,
            last_glyph_source_ordinal: None,
            last_emitted_vertex_end_count: None,
            cache_hit_replayed: false,
            cache_population_sealed: false,
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
        if active.cache_hit_replayed || active.cache_population_sealed {
            return Err("A25 font callback followed a sealed glyph-cache replay".into());
        }
        if let Some(existing) = active
            .fonts
            .iter()
            .find(|existing| existing.identity == receipt.identity)
        {
            return (existing == &receipt)
                .then_some(())
                .ok_or_else(|| "A25 font identity was rebound within one owner render".into());
        }
        active.fonts.push(receipt);
        Ok(())
    }

    pub fn record_inline_icon_asset(
        &mut self,
        receipt: InlineIconAssetReceipt,
    ) -> Result<(), String> {
        let active = self.active_mut()?;
        if active.cache_hit_replayed || active.cache_population_sealed {
            return Err("A25 icon-asset callback followed a sealed glyph-cache replay".into());
        }
        if let Some(existing) = active
            .icon_assets
            .iter()
            .find(|existing| existing.identity == receipt.identity)
        {
            return (existing == &receipt)
                .then_some(())
                .ok_or_else(|| "A25 inline-icon identity was rebound within one owner render".into());
        }
        active.icon_assets.push(receipt);
        Ok(())
    }

    /// Seal the current owner invocation's ordinary glyph runs to the exact
    /// cache entry populated by the engine. The native callback must run only
    /// after upload and after entry `+0`, `+0x10`, `+0x14`, and `+0x18` contain
    /// their final values. A previously occupied slot must first be removed by
    /// `invalidate_glyph_buffer_cache_slot` at the observed key-zero seam.
    pub fn seal_glyph_buffer_cache_population(
        &mut self,
        identity: GlyphBufferCacheIdentity,
    ) -> Result<(), String> {
        let (selected_final_text_sha256, glyphs, fonts) = {
            let active = self.active_mut()?;
            if active.cache_hit_replayed || active.cache_population_sealed {
                return Err("A25 glyph-cache population is duplicate or follows a hit".into());
            }
            if !active.inline_icons.is_empty() {
                return Err("A25 glyph-cache population contains non-cacheable inline icons".into());
            }
            (
                active.owner_join.selected_final_text_sha256.clone(),
                active.glyphs.clone(),
                active.fonts.clone(),
            )
        };
        self.glyph_buffer_cache_sidecars.seal_population(
            identity,
            selected_final_text_sha256,
            glyphs,
            &fonts,
        )?;
        self.active_mut()?.cache_population_sealed = true;
        Ok(())
    }

    /// Replay a previously sealed cache population into the current owner
    /// invocation and mirror exact-a729's unconditional full-entry swap with
    /// slot zero. `identity_before_hit` must describe the entry in its pre-swap
    /// slot at the hit branch; passing its post-swap slot is rejected.
    pub fn record_glyph_buffer_cache_hit(
        &mut self,
        identity_before_hit: GlyphBufferCacheIdentity,
    ) -> Result<(), String> {
        // Mirror the engine reorder first. It occurs on every hit regardless
        // of whether this capture can replay the corresponding sidecar.
        let sealed = self
            .glyph_buffer_cache_sidecars
            .observe_hit_reorder(&identity_before_hit)?;
        let (selected_final_text_sha256, existing_fonts, destination_rect) = {
            let active = self.active_mut()?;
            if active.cache_hit_replayed
                || active.cache_population_sealed
                || !active.glyphs.is_empty()
                || !active.inline_icons.is_empty()
                || active.last_glyph_source_ordinal.is_some()
                || active.last_emitted_vertex_end_count.is_some()
            {
                return Err(
                    "A25 glyph-cache hit is reordered with another glyph/icon decision".into(),
                );
            }
            (
                active.owner_join.selected_final_text_sha256.clone(),
                active.fonts.clone(),
                active.destination_rect,
            )
        };
        let sealed = sealed.ok_or("A25 glyph-cache hit references an unseen or evicted slot")?;
        if sealed.selected_final_text_sha256 != selected_final_text_sha256 {
            return Err("A25 glyph-cache hit selected-text identity mismatch".into());
        }
        let mut matching_existing_fonts = existing_fonts
            .iter()
            .filter(|receipt| receipt.identity == sealed.font_asset.identity);
        let existing_font = matching_existing_fonts.next();
        if matching_existing_fonts.next().is_some() {
            return Err("A25 glyph-cache hit found duplicate active font receipts".into());
        }
        if existing_font.is_some_and(|receipt| receipt != &sealed.font_asset) {
            return Err("A25 glyph-cache hit conflicts with the active font receipt".into());
        }
        let glyphs = rebase_cached_glyphs(sealed.glyphs, destination_rect)?;

        let active = self.active_mut()?;
        if existing_font.is_none() {
            active.fonts.push(sealed.font_asset);
        }
        active.glyphs = glyphs;
        active.cache_hit_replayed = true;
        Ok(())
    }

    /// Remove one exact sidecar when the engine zeroes entry `+0` while
    /// selecting a stale slot. A key/buffer/count/page mismatch cannot evict a
    /// different sidecar.
    pub fn invalidate_glyph_buffer_cache_slot(
        &mut self,
        identity_before_zero: &GlyphBufferCacheIdentity,
    ) -> Result<(), String> {
        self.validate_cache_callback_thread()?;
        self.glyph_buffer_cache_sidecars
            .evict_exact(identity_before_zero)
    }

    /// Font-destructor seam. This prevents allocator-address reuse from
    /// inheriting any sidecar belonging to the destroyed font object.
    pub fn teardown_glyph_buffer_cache_font(
        &mut self,
        font_instance_token: u64,
        font_identity: &str,
    ) -> Result<(), String> {
        self.validate_cache_callback_thread()?;
        self.glyph_buffer_cache_sidecars
            .clear_font(font_instance_token, font_identity)
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
        if active.cache_hit_replayed || active.cache_population_sealed {
            return Err("A25 ordinary-glyph callback followed a sealed cache replay".into());
        }
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

    /// Admit one exact post-emit callback. Position and color are decoded from
    /// the authenticated 28-byte GUI vertices, so a hook caller cannot stamp
    /// conflicting semantic values beside the raw receipt.
    pub fn record_post_emit_ordinary_glyph(
        &mut self,
        observation: PostEmitOrdinaryGlyphObservation,
    ) -> Result<(), String> {
        if observation.font_identity.trim().is_empty() {
            return Err("A25 post-emit glyph has no physical font identity".into());
        }
        if char::from_u32(observation.requested_codepoint).is_none()
            || char::from_u32(observation.resolved_codepoint).is_none()
        {
            return Err("A25 post-emit glyph codepoint is not a Unicode scalar".into());
        }
        if observation.descriptor_sha256.len() != 64
            || !observation
                .descriptor_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("A25 post-emit glyph descriptor hash is invalid".into());
        }
        if observation.emitted_vertex_end_count < 6
            || observation.emitted_vertex_end_count % 6 != 0
            || observation.generated_vertices.len()
                != crate::owner_glyph_capture::GLYPH_VERTEX_BYTES
        {
            return Err("A25 post-emit glyph vertex boundary is invalid".into());
        }

        let active = self.active_mut()?;
        if active.cache_hit_replayed || active.cache_population_sealed {
            return Err("A25 post-emit callback followed a sealed cache replay".into());
        }
        if active
            .last_glyph_source_ordinal
            .is_some_and(|previous| observation.source_ordinal <= previous)
        {
            return Err("A25 post-emit glyph source ordinal is duplicate or out of order".into());
        }
        let selected_text = std::str::from_utf8(&active.owner_join.selected_final_text_bytes)
            .map_err(|_| "A25 selected final text is not strict UTF-8".to_string())?;
        let selected_codepoint = selected_text
            .chars()
            .nth(observation.source_ordinal as usize)
            .map(u32::from)
            .ok_or("A25 post-emit glyph source ordinal exceeds selected final text")?;
        if selected_codepoint != observation.requested_codepoint {
            return Err("A25 post-emit glyph codepoint disagrees with selected source ordinal".into());
        }
        if let Some(previous) = active.last_emitted_vertex_end_count {
            let contiguous = previous
                .checked_add(6)
                .is_some_and(|next| observation.emitted_vertex_end_count == next);
            let exact_flush_reset =
                previous == 3072 && observation.emitted_vertex_end_count == 6;
            if !contiguous && !exact_flush_reset {
                return Err("A25 post-emit glyph vertex count skipped or replayed a group".into());
            }
        } else if observation.emitted_vertex_end_count != 6 {
            return Err("A25 first post-emit glyph does not begin a scratch partition".into());
        }
        let expected_group_start = crate::owner_glyph_capture::GLYPH_VERTEX_SCRATCH_RVA
            .checked_add(
                observation
                    .emitted_vertex_end_count
                    .checked_sub(6)
                    .and_then(|count| {
                        count.checked_mul(crate::owner_glyph_capture::GLYPH_VERTEX_STRIDE as u32)
                    })
                    .ok_or("A25 post-emit glyph scratch offset overflow")?,
            )
            .ok_or("A25 post-emit glyph scratch RVA overflow")?;
        if observation.scratch_group_start_rva != expected_group_start {
            return Err("A25 post-emit glyph bytes are not bound to the exact scratch group".into());
        }
        let mut min_x = None::<f32>;
        let mut min_y = None::<f32>;
        let mut packed_color = None::<u32>;
        for vertex in observation
            .generated_vertices
            .chunks_exact(crate::owner_glyph_capture::GLYPH_VERTEX_STRIDE)
        {
            let word = |offset: usize| {
                u32::from_le_bytes(vertex[offset..offset + 4].try_into().unwrap())
            };
            let x = f32::from_bits(word(0));
            let y = f32::from_bits(word(4));
            if !x.is_finite() || !y.is_finite() {
                return Err("A25 post-emit glyph position is non-finite".into());
            }
            min_x = Some(min_x.map_or(x, |value| value.min(x)));
            min_y = Some(min_y.map_or(y, |value| value.min(y)));
            let color = word(20);
            if packed_color.replace(color).is_some_and(|prior| prior != color) {
                return Err("A25 post-emit glyph vertices disagree on color".into());
            }
        }
        let exact_i32 = |value: f32, label: &str| -> Result<i32, String> {
            if value < i32::MIN as f32 || value > i32::MAX as f32 || value.fract() != 0.0 {
                return Err(format!("A25 post-emit glyph {label} is not an exact integer"));
            }
            Ok(value as i32)
        };
        let absolute_x = exact_i32(min_x.ok_or("A25 post-emit glyph has no x")?, "x")?;
        let absolute_y = exact_i32(min_y.ok_or("A25 post-emit glyph has no y")?, "y")?;
        let owner_relative_x = absolute_x
            .checked_sub(active.destination_rect[0])
            .ok_or("A25 post-emit glyph owner-relative x overflow")?;
        let owner_relative_y = absolute_y
            .checked_sub(active.destination_rect[1])
            .ok_or("A25 post-emit glyph owner-relative y overflow")?;
        let bgra = packed_color.ok_or("A25 post-emit glyph has no color")?.to_le_bytes();
        let rgba = [bgra[2], bgra[1], bgra[0], bgra[3]];
        let descriptor = if observation.requested_codepoint == observation.resolved_codepoint {
            GlyphDescriptorIdentity::UnicodeScalar(observation.resolved_codepoint)
        } else {
            GlyphDescriptorIdentity::DescriptorSha256(observation.descriptor_sha256)
        };
        let run_ordinal = u32::try_from(active.glyphs.len() + active.inline_icons.len())
            .map_err(|_| "A25 glyph/icon run count exceeds u32".to_string())?;
        active.glyphs.push(OwnerScopedGlyphObservation {
            run_ordinal,
            font_identity: observation.font_identity,
            descriptor,
            glyph_id: observation.resolved_codepoint,
            owner_relative_x,
            owner_relative_y,
            rgba,
            generated_vertices: observation.generated_vertices,
        });
        active.last_glyph_source_ordinal = Some(observation.source_ordinal);
        active.last_emitted_vertex_end_count = Some(observation.emitted_vertex_end_count);
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
        if active.cache_hit_replayed || active.cache_population_sealed {
            return Err("A25 inline-icon callback followed a sealed cache replay".into());
        }
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

    /// Record the exact absolute coordinates accepted by
    /// `CGameBitmapFont::DrawInlineIcon`.  The owner destination is the sole
    /// coordinate-space authority; live absolute coordinates never escape
    /// into the portable replay payload.
    #[allow(clippy::too_many_arguments)]
    pub fn record_inline_icon_absolute(
        &mut self,
        token: String,
        sprite_identity: String,
        frame: u32,
        absolute_x: i32,
        absolute_y: i32,
        advance: i32,
        draw_receipt_sha256: String,
    ) -> Result<(), String> {
        let active = self.active_mut()?;
        let owner_relative_x = absolute_x
            .checked_sub(active.destination_rect[0])
            .ok_or("A25 inline-icon owner-relative x overflow")?;
        let owner_relative_y = absolute_y
            .checked_sub(active.destination_rect[1])
            .ok_or("A25 inline-icon owner-relative y overflow")?;
        self.record_inline_icon(
            token,
            sprite_identity,
            frame,
            owner_relative_x,
            owner_relative_y,
            owner_relative_y,
            advance,
            draw_receipt_sha256,
        )
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
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.owner_thread_id != thread::current().id())
        {
            self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
            if let Some(active) = self.active.as_mut() {
                active.health.hook_failure_count =
                    active.health.hook_failure_count.saturating_add(1);
            }
            return Err("A25 owner-glyph render exit crossed hook threads".into());
        }
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
            gui_engine_instance_token: active.gui_engine_instance_token,
            paint_ordinal: active.paint_ordinal,
            destination_rect: active.destination_rect,
            ui_clip: active.ui_clip,
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
        let reservation = self.reserve_selected_present(selected, frame_hour)?;
        self.commit_selected_present(reservation.reservation_id)?;
        Ok(reservation.captures)
    }

    fn reserve_selected_present(
        &mut self,
        selected: OwnerGlyphPresentCandidate,
        frame_hour: u64,
    ) -> Result<SelectedOwnerGlyphCaptureReservation, String> {
        self.validate_candidate(selected, frame_hour)?;
        if self.active.is_some() {
            return Err("A25 selected-Present drain crossed an active owner render".into());
        }
        if let Some(reservation) = self.selection_reservation.as_mut() {
            if reservation.active {
                return Err("another A25 selected-Present owner reservation is active".into());
            }
            if reservation.selected != selected {
                return Err(
                    "retained A25 selected-Present owners belong to another candidate".into(),
                );
            }
            let reservation_id = self.next_selection_reservation_id;
            self.next_selection_reservation_id = reservation_id
                .checked_add(1)
                .ok_or("A25 owner-glyph selection reservation id exhausted")?;
            reservation.reservation_id = reservation_id;
            reservation.active = true;
            return Ok(SelectedOwnerGlyphCaptureReservation {
                reservation_id,
                captures: reservation.captures.clone(),
            });
        }
        let reservation_id = self.next_selection_reservation_id;
        self.next_selection_reservation_id = reservation_id
            .checked_add(1)
            .ok_or("A25 owner-glyph selection reservation id exhausted")?;
        let mut admission_after = self.admission.clone();
        let mut admitted = Vec::new();
        let mut installs = Vec::new();
        let mut future = VecDeque::new();
        let mut discarded_non_selected_count = 0u64;
        for completed in &self.completed {
            if completed.candidate.frame_hour > selected.frame_hour {
                future.push_back(completed.clone());
                continue;
            }
            let selected_capture = completed.candidate == selected;
            let admitted_capture = admission_after.admit_against_identity(
                completed.capture.clone(),
                &completed.row_identity,
                completed.candidate.frame_hour,
            )?;
            if !selected_capture {
                discarded_non_selected_count =
                    discarded_non_selected_count.saturating_add(1);
                continue;
            }
            installs.push(SelectedOwnerInstall {
                owner: completed.owner,
                context_id: completed.row_identity.context_id.clone(),
                selected_final_text_sha256: completed
                    .row_identity
                    .selected_final_text_sha256
                    .clone(),
                admitted: admitted_capture.clone(),
            });
            admitted.push(SelectedOwnerGlyphCapture {
                candidate: selected,
                context_id: completed.row_identity.context_id.clone(),
                view_id: completed.row_identity.view_id.clone(),
                element_id: completed.row_identity.element_id.clone(),
                admitted: admitted_capture,
            });
        }
        self.completed = future;
        self.selection_reservation = Some(OwnerGlyphSelectionReservation {
            reservation_id,
            selected,
            captures: admitted.clone(),
            installs,
            admission_after,
            discarded_non_selected_count,
            active: true,
        });
        Ok(SelectedOwnerGlyphCaptureReservation {
            reservation_id,
            captures: admitted,
        })
    }

    fn commit_selected_present(&mut self, reservation_id: u64) -> Result<(), String> {
        let reservation = self
            .selection_reservation
            .as_ref()
            .ok_or("A25 selected-Present owner reservation is absent")?;
        if !reservation.active || reservation.reservation_id != reservation_id {
            return Err("A25 selected-Present owner reservation id is stale".into());
        }
        for install in &reservation.installs {
            install_admitted_owner_capture(
                install.owner,
                &install.context_id,
                &install.selected_final_text_sha256,
                &install.admitted,
            )?;
        }
        self.admission = reservation.admission_after.clone();
        self.telemetry.discarded_non_selected_count = self
            .telemetry
            .discarded_non_selected_count
            .saturating_add(reservation.discarded_non_selected_count);
        self.selection_reservation = None;
        Ok(())
    }

    fn rollback_selected_present(&mut self, reservation_id: u64) -> Result<(), String> {
        let reservation = self
            .selection_reservation
            .as_mut()
            .ok_or("A25 selected-Present owner reservation is absent")?;
        if !reservation.active || reservation.reservation_id != reservation_id {
            return Err("A25 selected-Present owner reservation id is stale".into());
        }
        reservation.active = false;
        Ok(())
    }

    pub fn telemetry(&self) -> OwnerGlyphProbeTelemetry {
        self.telemetry
    }

    fn active_mut(&mut self) -> Result<&mut ActiveInvocation, String> {
        if self.active.is_none()
            || self
                .active
                .as_ref()
                .is_some_and(|active| active.owner_thread_id != thread::current().id())
        {
            self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.owner_thread_id != thread::current().id())
        {
            if let Some(active) = self.active.as_mut() {
                active.health.hook_failure_count =
                    active.health.hook_failure_count.saturating_add(1);
            }
            return Err("A25 glyph/icon callback crossed hook threads".into());
        }
        self.active
            .as_mut()
            .ok_or_else(|| "A25 glyph/icon callback is outside an owner render".to_string())
    }

    fn note_active_failure(&mut self, note: impl FnOnce(&mut CaptureHealth)) {
        if let Some(active) = self.active.as_mut() {
            if active.owner_thread_id == thread::current().id() {
                note(&mut active.health);
            } else {
                active.health.hook_failure_count =
                    active.health.hook_failure_count.saturating_add(1);
                self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
            }
        } else {
            self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
        }
    }

    fn validate_cache_callback_thread(&mut self) -> Result<(), String> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.owner_thread_id != thread::current().id())
        {
            self.telemetry.rejected_count = self.telemetry.rejected_count.saturating_add(1);
            if let Some(active) = self.active.as_mut() {
                active.health.hook_failure_count =
                    active.health.hook_failure_count.saturating_add(1);
            }
            return Err("A25 glyph-cache callback crossed hook threads".into());
        }
        Ok(())
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
    OWNER_GLYPH_AUTOMATION.get_or_init(|| Mutex::new(None))
}

/// Install the passive production owner/glyph state machine for one exact
/// render-hook epoch. Unlike the historical evidence switch, this API is
/// authorized by the authenticated production-render bootstrap and requires
/// the same nonzero process/hook identities later carried by the selected
/// Present. It installs no client driver and performs no active interaction.
pub fn install_production_automation(
    process_identity: String,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
) -> Result<(), String> {
    let automation = OwnerGlyphEvidenceAutomation::production(
        process_identity,
        process_start_nonce,
        hook_epoch_nonce,
    )?;
    let mut slot = automation_slot()
        .lock()
        .map_err(|_| "A25 owner-glyph automation state poisoned".to_string())?;
    if slot.is_some() {
        return Err("A25 owner-glyph automation is already installed".into());
    }
    *slot = Some(automation);
    Ok(())
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

    #[test]
    fn production_automation_is_epoch_bound_and_distinct_from_evidence_mode() {
        install_production_automation("exact-a729-production".into(), 7, 11).unwrap();
        assert!(production_automation_is_active());
        let selected = OwnerGlyphPresentCandidate {
            process_start_nonce: 7,
            hook_epoch_nonce: 11,
            submit_index: 1,
            frame_sequence: 1,
            frame_hour: 42,
        };
        assert_eq!(
            admit_selected_present_if_production_active(selected).unwrap(),
            Some(Vec::new())
        );
        assert!(with_dormant_evidence_automation(|_| Ok(())).unwrap().is_none());
        let telemetry = uninstall_production_automation().unwrap();
        assert_eq!(telemetry, OwnerGlyphProbeTelemetry::default());
        assert!(!production_automation_is_active());
    }
}

#[cfg(test)]
mod post_emit_tests {
    use super::*;
    use crate::owner_glyph_capture::{
        GlyphDescriptorIdentity, GLYPH_VERTEX_BYTES, GLYPH_VERTEX_SCRATCH_RVA,
        GLYPH_VERTEX_STRIDE,
    };
    use sha2::{Digest, Sha256};

    fn automation_with_active_text(text: &str) -> OwnerGlyphEvidenceAutomation {
        let mut automation =
            OwnerGlyphEvidenceAutomation::dormant("exact-a729-test".into(), 1, 1).unwrap();
        let selected_final_text_bytes = text.as_bytes().to_vec();
        let selected_final_text_sha256 =
            format!("{:x}", Sha256::digest(&selected_final_text_bytes));
        automation.active = Some(ActiveInvocation {
            owner_thread_id: thread::current().id(),
            owner: 0x1234,
            owner_join: ObservedOwnerRenderJoin {
                owner: 0x1234,
                context_id: "view::element::0".into(),
                view_id: "view".into(),
                element_id: "element".into(),
                source: crate::RuntimeTextSourceIdentity::AuthoredToken("TOKEN".into()),
                selected_final_text_sha256,
                selected_final_text_bytes,
            },
            candidate: OwnerGlyphPresentCandidate {
                process_start_nonce: 1,
                hook_epoch_nonce: 1,
                submit_index: 1,
                frame_sequence: 1,
                frame_hour: 1,
            },
            frame_hour: 1,
            render_invocation_ordinal: 0,
            gui_engine_instance_token: 1,
            paint_ordinal: 0,
            destination_rect: [10, 20, 110, 60],
            ui_clip: None,
            clip: None,
            last_glyph_source_ordinal: None,
            last_emitted_vertex_end_count: None,
            cache_hit_replayed: false,
            cache_population_sealed: false,
            glyphs: Vec::new(),
            inline_icons: Vec::new(),
            fonts: Vec::new(),
            icon_assets: Vec::new(),
            health: CaptureHealth::default(),
        });
        automation
    }

    fn vertices(x: f32, y: f32, bgra: [u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0u8; GLYPH_VERTEX_BYTES];
        for vertex in bytes.chunks_exact_mut(GLYPH_VERTEX_STRIDE) {
            vertex[0..4].copy_from_slice(&x.to_le_bytes());
            vertex[4..8].copy_from_slice(&y.to_le_bytes());
            vertex[20..24].copy_from_slice(&u32::from_le_bytes(bgra).to_le_bytes());
        }
        bytes
    }

    fn observation(
        codepoint: char,
        source_ordinal: u32,
        end_count: u32,
    ) -> PostEmitOrdinaryGlyphObservation {
        PostEmitOrdinaryGlyphObservation {
            font_identity: "gfx/fonts/hoi_16b.fnt".into(),
            requested_codepoint: codepoint as u32,
            resolved_codepoint: codepoint as u32,
            descriptor_sha256: "a".repeat(64),
            source_ordinal,
            emitted_vertex_end_count: end_count,
            scratch_group_start_rva: GLYPH_VERTEX_SCRATCH_RVA
                + (end_count - 6) * GLYPH_VERTEX_STRIDE as u32,
            generated_vertices: vertices(15.0, 27.0, [3, 2, 1, 4]),
        }
    }

    #[test]
    fn post_emit_happy_path_binds_owner_relative_geometry_and_color() {
        let mut automation = automation_with_active_text("A");
        automation
            .record_post_emit_ordinary_glyph(observation('A', 0, 6))
            .unwrap();

        let active = automation.active.as_ref().unwrap();
        assert_eq!(active.last_glyph_source_ordinal, Some(0));
        assert_eq!(active.last_emitted_vertex_end_count, Some(6));
        assert_eq!(active.glyphs.len(), 1);
        let glyph = &active.glyphs[0];
        assert_eq!(glyph.run_ordinal, 0);
        assert_eq!(
            glyph.descriptor,
            GlyphDescriptorIdentity::UnicodeScalar('A' as u32)
        );
        assert_eq!((glyph.owner_relative_x, glyph.owner_relative_y), (5, 7));
        assert_eq!(glyph.rgba, [1, 2, 3, 4]);
        assert_eq!(glyph.generated_vertices.len(), GLYPH_VERTEX_BYTES);
    }

    #[test]
    fn post_emit_duplicate_and_out_of_order_ordinals_fail_closed() {
        for rejected_ordinal in [1, 0] {
            let mut automation = automation_with_active_text("ABC");
            automation
                .record_post_emit_ordinary_glyph(observation('B', 1, 6))
                .unwrap();
            let error = automation
                .record_post_emit_ordinary_glyph(observation(
                    if rejected_ordinal == 0 { 'A' } else { 'B' },
                    rejected_ordinal,
                    12,
                ))
                .expect_err("non-increasing source ordinal must fail");
            assert!(error.contains("duplicate or out of order"), "{error}");
            assert_eq!(automation.active.as_ref().unwrap().glyphs.len(), 1);
        }
    }

    #[test]
    fn post_emit_codepoint_mismatch_fails_closed() {
        let mut automation = automation_with_active_text("A");
        let error = automation
            .record_post_emit_ordinary_glyph(observation('B', 0, 6))
            .expect_err("source text/codepoint mismatch must fail");
        assert!(error.contains("codepoint disagrees"), "{error}");
        assert!(automation.active.as_ref().unwrap().glyphs.is_empty());
    }

    #[test]
    fn post_emit_counter_skip_fails_closed() {
        let mut automation = automation_with_active_text("AB");
        automation
            .record_post_emit_ordinary_glyph(observation('A', 0, 6))
            .unwrap();
        let error = automation
            .record_post_emit_ordinary_glyph(observation('B', 1, 18))
            .expect_err("skipped six-vertex group must fail");
        assert!(error.contains("vertex count skipped"), "{error}");
        assert_eq!(automation.active.as_ref().unwrap().glyphs.len(), 1);
    }

    #[test]
    fn post_emit_exact_3072_to_6_flush_reset_is_admitted() {
        let mut automation = automation_with_active_text("AB");
        let active = automation.active.as_mut().unwrap();
        active.last_glyph_source_ordinal = Some(0);
        active.last_emitted_vertex_end_count = Some(3072);

        automation
            .record_post_emit_ordinary_glyph(observation('B', 1, 6))
            .expect("the exact scratch-capacity reset is authoritative");
        let active = automation.active.as_ref().unwrap();
        assert_eq!(active.last_emitted_vertex_end_count, Some(6));
        assert_eq!(active.glyphs.len(), 1);
    }

    #[test]
    fn post_emit_wrong_scratch_rva_fails_closed() {
        let mut automation = automation_with_active_text("A");
        let mut row = observation('A', 0, 6);
        row.scratch_group_start_rva += GLYPH_VERTEX_STRIDE as u32;
        let error = automation
            .record_post_emit_ordinary_glyph(row)
            .expect_err("wrong scratch group address must fail");
        assert!(error.contains("exact scratch group"), "{error}");
        assert!(automation.active.as_ref().unwrap().glyphs.is_empty());
    }

    #[test]
    fn post_emit_cross_thread_callback_is_rejected_without_appending() {
        let automation = automation_with_active_text("A");
        let (automation, error) = std::thread::spawn(move || {
            let mut automation = automation;
            let error = automation
                .record_post_emit_ordinary_glyph(observation('A', 0, 6))
                .expect_err("a callback on a non-owner hook thread must fail closed");
            (automation, error)
        })
        .join()
        .expect("cross-thread rejection test must join");

        assert!(error.contains("crossed hook threads"), "{error}");
        let active = automation.active.as_ref().unwrap();
        assert!(active.glyphs.is_empty());
        assert_eq!(active.health.hook_failure_count, 1);
        assert_eq!(automation.telemetry().rejected_count, 1);
    }

    #[test]
    fn cache_population_and_exact_hit_replay_integrate_with_owner_automation() {
        let mut automation = automation_with_active_text("A");
        automation
            .record_font_asset(FontAssetReceipt {
                identity: "gfx/fonts/hoi_16b.fnt".into(),
                descriptor_sha256: "a".repeat(64),
                atlas_sha256: vec!["b".repeat(64)],
            })
            .unwrap();
        automation
            .record_post_emit_ordinary_glyph(observation('A', 0, 6))
            .unwrap();
        let identity = GlyphBufferCacheIdentity {
            font_instance_token: 0x1111,
            font_identity: "gfx/fonts/hoi_16b.fnt".into(),
            slot_index: 7,
            cache_key: 0x1234,
            texture_or_page_id: 3,
            vertex_count: 6,
            buffer_identity: 0x2222,
        };
        automation
            .seal_glyph_buffer_cache_population(identity.clone())
            .unwrap();
        assert!(automation.active.as_ref().unwrap().cache_population_sealed);
        assert!(automation
            .seal_glyph_buffer_cache_population(identity.clone())
            .is_err());
        assert!(automation
            .record_post_emit_ordinary_glyph(observation('A', 0, 12))
            .is_err());

        automation.active = automation_with_active_text("A").active;
        automation.active.as_mut().unwrap().destination_rect = [12, 22, 112, 62];
        automation
            .record_glyph_buffer_cache_hit(identity.clone())
            .unwrap();
        let active = automation.active.as_ref().unwrap();
        assert!(active.cache_hit_replayed);
        assert_eq!(active.glyphs.len(), 1);
        assert_eq!(active.fonts.len(), 1);
        assert_eq!(
            (active.glyphs[0].owner_relative_x, active.glyphs[0].owner_relative_y),
            (3, 5)
        );

        // The engine moved the hit entry to slot zero. Its genuine post-swap
        // slot is admissible exactly once before any contradictory callback.
        automation.active = automation_with_active_text("A").active;
        automation
            .record_glyph_buffer_cache_hit(identity.at_slot(0))
            .unwrap();

        // Supplying the stale pre-hit slot now proves chronology corruption,
        // clears the font sidecar, and cannot move the tuple back to slot 7.
        automation.active = automation_with_active_text("A").active;
        assert!(automation
            .record_glyph_buffer_cache_hit(identity.clone())
            .is_err());
        assert!(!automation
            .glyph_buffer_cache_sidecars
            .fonts
            .contains_key(&identity.font_instance_token));

        automation.active = automation_with_active_text("A").active;
        assert!(automation
            .record_glyph_buffer_cache_hit(identity.at_slot(0))
            .is_err());
    }

    #[test]
    fn rejected_text_identity_still_mirrors_observed_cache_reorder() {
        let mut automation = automation_with_active_text("A");
        automation
            .record_font_asset(FontAssetReceipt {
                identity: "gfx/fonts/hoi_16b.fnt".into(),
                descriptor_sha256: "a".repeat(64),
                atlas_sha256: vec!["b".repeat(64)],
            })
            .unwrap();
        automation
            .record_post_emit_ordinary_glyph(observation('A', 0, 6))
            .unwrap();
        let identity = GlyphBufferCacheIdentity {
            font_instance_token: 0x1111,
            font_identity: "gfx/fonts/hoi_16b.fnt".into(),
            slot_index: 7,
            cache_key: 0x1234,
            texture_or_page_id: 3,
            vertex_count: 6,
            buffer_identity: 0x2222,
        };
        automation
            .seal_glyph_buffer_cache_population(identity.clone())
            .unwrap();

        automation.active = automation_with_active_text("B").active;
        assert!(automation
            .record_glyph_buffer_cache_hit(identity.clone())
            .is_err());
        automation.active = automation_with_active_text("A").active;
        automation
            .record_glyph_buffer_cache_hit(identity.at_slot(0))
            .expect("text rejection must not leave the sidecar in the pre-hit slot");
    }
}

#[cfg(test)]
mod cache_sidecar_tests {
    use super::*;

    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn identity(font: u64, slot: u8, key: i32, buffer: u64) -> GlyphBufferCacheIdentity {
        GlyphBufferCacheIdentity {
            font_instance_token: font,
            font_identity: format!("font-{font:x}"),
            slot_index: slot,
            cache_key: key,
            texture_or_page_id: 5,
            vertex_count: 6,
            buffer_identity: buffer,
        }
    }

    fn glyph(font_identity: &str) -> OwnerScopedGlyphObservation {
        OwnerScopedGlyphObservation {
            run_ordinal: 0,
            font_identity: font_identity.into(),
            descriptor: GlyphDescriptorIdentity::UnicodeScalar('A' as u32),
            glyph_id: 'A' as u32,
            owner_relative_x: 1,
            owner_relative_y: 2,
            rgba: [255; 4],
            generated_vertices: vec![0; crate::owner_glyph_capture::GLYPH_VERTEX_BYTES],
        }
    }

    fn font_asset(font_identity: &str) -> FontAssetReceipt {
        FontAssetReceipt {
            identity: font_identity.into(),
            descriptor_sha256: DIGEST.into(),
            atlas_sha256: vec![DIGEST.into()],
        }
    }

    fn seal(cache: &mut GlyphBufferCacheSidecars, identity: GlyphBufferCacheIdentity) {
        cache
            .seal_population(
                identity.clone(),
                DIGEST.into(),
                vec![glyph(&identity.font_identity)],
                &[font_asset(&identity.font_identity)],
            )
            .unwrap();
    }

    #[test]
    fn exact_hit_mirrors_full_entry_swap_with_slot_zero() {
        let mut cache = GlyphBufferCacheSidecars::default();
        let zero = identity(1, 0, 10, 100);
        let hit = identity(1, 7, 20, 200);
        seal(&mut cache, zero.clone());
        seal(&mut cache, hit.clone());

        let payload = cache.observe_hit_reorder(&hit).unwrap().unwrap();
        assert_eq!(payload.identity, hit);
        let font = cache.fonts.get(&1).unwrap();
        assert_eq!(font.slots[0].as_ref().unwrap().identity, hit.at_slot(0));
        assert_eq!(font.slots[7].as_ref().unwrap().identity, zero.at_slot(7));
    }

    #[test]
    fn unseen_mismatch_overwrite_eviction_and_reorder_fail_closed() {
        let mut cache = GlyphBufferCacheSidecars::default();
        let observed = identity(1, 4, 20, 200);
        assert!(cache.observe_hit_reorder(&observed).unwrap().is_none());
        seal(&mut cache, observed.clone());

        for mutate in [0, 1, 2] {
            let mut isolated = GlyphBufferCacheSidecars::default();
            seal(&mut isolated, observed.clone());
            let mut mismatch = observed.clone();
            match mutate {
                0 => mismatch.texture_or_page_id += 1,
                1 => mismatch.vertex_count = 12,
                _ => mismatch.buffer_identity += 1,
            }
            assert!(isolated.observe_hit_reorder(&mismatch).is_err());
            assert!(!isolated.fonts.contains_key(&1));
        }

        let replacement = identity(1, 4, 21, 201);
        assert!(cache
            .seal_population(
                replacement.clone(),
                DIGEST.into(),
                vec![glyph(&replacement.font_identity)],
                &[font_asset(&replacement.font_identity)],
            )
            .is_err());
        let mut wrong_eviction = observed.clone();
        wrong_eviction.buffer_identity += 1;
        assert!(cache.evict_exact(&wrong_eviction).is_err());
        cache.evict_exact(&observed).unwrap();
        seal(&mut cache, replacement.clone());
        assert!(cache.observe_hit_reorder(&observed).is_err());
    }

    #[test]
    fn unseen_hit_still_displaces_tracked_slot_zero() {
        let mut cache = GlyphBufferCacheSidecars::default();
        let zero = identity(1, 0, 10, 100);
        let unseen = identity(1, 7, 20, 200);
        seal(&mut cache, zero.clone());

        assert!(cache.observe_hit_reorder(&unseen).unwrap().is_none());
        let font = cache.fonts.get(&1).unwrap();
        assert!(font.slots[0].is_none());
        assert_eq!(font.slots[7].as_ref().unwrap().identity, zero.at_slot(7));
    }

    #[test]
    fn each_font_is_bounded_to_40_slots_and_teardown_clears() {
        let mut cache = GlyphBufferCacheSidecars::default();
        for slot in 0..GLYPH_BUFFER_CACHE_SIDECAR_SLOT_COUNT as u8 {
            seal(
                &mut cache,
                identity(1, slot, i32::from(slot) + 1, u64::from(slot) + 1),
            );
            seal(
                &mut cache,
                identity(2, slot, i32::from(slot) + 101, u64::from(slot) + 101),
            );
        }
        assert_eq!(cache.fonts.len(), 2);
        assert!(cache
            .fonts
            .values()
            .all(|font| font.slots.len() == GLYPH_BUFFER_CACHE_SIDECAR_SLOT_COUNT));

        let outside = identity(1, GLYPH_BUFFER_CACHE_SIDECAR_SLOT_COUNT as u8, 99, 99);
        assert!(outside.validate().is_err());
        assert!(cache.clear_font(1, "font-2").is_err());
        cache.clear_font(1, "font-1").unwrap();
        assert!(!cache.fonts.contains_key(&1));
        cache.clear();
        assert!(cache.fonts.is_empty());
    }
}

pub fn uninstall_dormant_evidence_automation() -> Result<OwnerGlyphProbeTelemetry, String> {
    uninstall_automation(AutomationMode::Evidence)
}

pub fn uninstall_production_automation() -> Result<OwnerGlyphProbeTelemetry, String> {
    uninstall_automation(AutomationMode::Production)
}

pub fn production_automation_is_active() -> bool {
    automation_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|automation| automation.mode))
        == Some(AutomationMode::Production)
}

fn uninstall_automation(
    expected_mode: AutomationMode,
) -> Result<OwnerGlyphProbeTelemetry, String> {
    let mut slot = automation_slot()
        .lock()
        .map_err(|_| "A25 owner-glyph evidence automation state poisoned".to_string())?;
    let installed = slot
        .as_ref()
        .ok_or("A25 owner-glyph automation is not installed")?;
    if installed.mode != expected_mode {
        return Err("A25 owner-glyph automation mode differs during teardown".into());
    }
    if installed.active.is_some() {
        return Err("A25 owner-glyph automation teardown crossed an active render".into());
    }
    if installed.selection_reservation.is_some() {
        return Err(
            "A25 owner-glyph automation teardown refused with unresolved selected-Present custody"
                .into(),
        );
    }
    let mut automation = slot.take().expect("automation presence checked above");
    automation.glyph_buffer_cache_sidecars.clear();
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
    if automation.mode != AutomationMode::Evidence {
        return Ok(None);
    }
    operation(automation).map(Some)
}

fn with_active_automation<T>(
    operation: impl FnOnce(&mut OwnerGlyphEvidenceAutomation) -> Result<T, String>,
) -> Result<Option<T>, String> {
    let mut slot = automation_slot()
        .lock()
        .map_err(|_| "A25 owner-glyph automation state poisoned".to_string())?;
    let Some(automation) = slot.as_mut() else {
        return Ok(None);
    };
    operation(automation).map(Some)
}

pub fn with_production_automation<T>(
    operation: impl FnOnce(&mut OwnerGlyphEvidenceAutomation) -> Result<T, String>,
) -> Result<Option<T>, String> {
    let mut slot = automation_slot()
        .lock()
        .map_err(|_| "A25 owner-glyph production automation state poisoned".to_string())?;
    let Some(automation) = slot.as_mut() else {
        return Ok(None);
    };
    if automation.mode != AutomationMode::Production {
        return Ok(None);
    }
    operation(automation).map(Some)
}

#[allow(clippy::too_many_arguments)]
pub fn begin_production_owner_render(
    layout_object: usize,
    candidate: OwnerGlyphPresentCandidate,
    gui_engine_instance_token: u64,
    paint_ordinal: u32,
    destination_rect: [i32; 4],
    ui_clip: Option<[i32; 4]>,
    clip: Option<[i32; 4]>,
) -> Result<bool, String> {
    with_production_automation(|automation| {
        automation.begin_owner_render(
            layout_object,
            candidate,
            candidate.frame_hour,
            gui_engine_instance_token,
            paint_ordinal,
            destination_rect,
            ui_clip,
            clip,
        )
    })
    .map(|value| value.is_some())
}

pub fn finish_production_owner_render() -> Result<bool, String> {
    with_production_automation(|automation| automation.finish_owner_render())
        .map(|value| value.is_some())
}

pub fn record_production_font_asset(
    receipt: FontAssetReceipt,
) -> Result<bool, String> {
    with_production_automation(|automation| automation.record_font_asset(receipt))
        .map(|value| value.is_some())
}

pub fn record_production_inline_icon_asset(
    receipt: InlineIconAssetReceipt,
) -> Result<bool, String> {
    with_production_automation(|automation| automation.record_inline_icon_asset(receipt))
        .map(|value| value.is_some())
}

#[allow(clippy::too_many_arguments)]
pub fn record_production_inline_icon(
    token: String,
    sprite_identity: String,
    frame: u32,
    owner_relative_x: i32,
    owner_relative_y: i32,
    baseline: i32,
    advance: i32,
    draw_receipt_sha256: String,
) -> Result<bool, String> {
    with_production_automation(|automation| {
        automation.record_inline_icon(
            token,
            sprite_identity,
            frame,
            owner_relative_x,
            owner_relative_y,
            baseline,
            advance,
            draw_receipt_sha256,
        )
    })
    .map(|value| value.is_some())
}

#[allow(clippy::too_many_arguments)]
pub fn record_production_inline_icon_absolute(
    token: String,
    sprite_identity: String,
    frame: u32,
    absolute_x: i32,
    absolute_y: i32,
    advance: i32,
    draw_receipt_sha256: String,
) -> Result<bool, String> {
    with_production_automation(|automation| {
        automation.record_inline_icon_absolute(
            token,
            sprite_identity,
            frame,
            absolute_x,
            absolute_y,
            advance,
            draw_receipt_sha256,
        )
    })
    .map(|value| value.is_some())
}

/// Passive native-hook insertion API for exact-a729 `0x14228C585`. Returns
/// `false` when A25 capture is not installed; this never fabricates an empty
/// observation. Cache-hit paths must replay only a previously sealed sidecar
/// for the exact 40-entry cache slot/key/buffer tuple, otherwise remain
/// fail-closed until the engine repopulates that slot.
pub fn observe_post_emit_ordinary_glyph_if_active(
    observation: PostEmitOrdinaryGlyphObservation,
) -> Result<bool, String> {
    with_active_automation(|automation| {
        automation.record_post_emit_ordinary_glyph(observation)
    })
    .map(|recorded| recorded.is_some())
}

/// Hook-facing population seam. Call only after the exact-a729 upload path has
/// finalized this slot's key, texture/page id, vertex count, and GPU buffer.
pub fn observe_glyph_buffer_cache_population_if_active(
    identity: GlyphBufferCacheIdentity,
) -> Result<bool, String> {
    with_active_automation(|automation| {
        automation.seal_glyph_buffer_cache_population(identity)
    })
    .map(|recorded| recorded.is_some())
}

/// Hook-facing hit seam. `identity_before_hit` is the exact tuple at the slot
/// found by the key scan before the function swaps that entry with slot zero.
pub fn observe_glyph_buffer_cache_hit_if_active(
    identity_before_hit: GlyphBufferCacheIdentity,
) -> Result<bool, String> {
    with_active_automation(|automation| {
        automation.record_glyph_buffer_cache_hit(identity_before_hit)
    })
    .map(|recorded| recorded.is_some())
}

/// Hook-facing stale-entry seam. Call with the complete old tuple immediately
/// before or after the engine writes zero to entry `+0` and before overwrite.
pub fn observe_glyph_buffer_cache_zero_invalidation_if_active(
    identity_before_zero: GlyphBufferCacheIdentity,
) -> Result<bool, String> {
    with_active_automation(|automation| {
        automation.invalidate_glyph_buffer_cache_slot(&identity_before_zero)
    })
    .map(|recorded| recorded.is_some())
}

/// Hook-facing font destructor seam. Exact token plus physical font identity
/// prevents a reused allocator address from inheriting stale glyph runs.
pub fn observe_glyph_buffer_cache_font_teardown_if_active(
    font_instance_token: u64,
    font_identity: String,
) -> Result<bool, String> {
    with_active_automation(|automation| {
        automation.teardown_glyph_buffer_cache_font(font_instance_token, &font_identity)
    })
    .map(|recorded| recorded.is_some())
}

pub fn admit_selected_present_if_evidence_active(
    selected: OwnerGlyphPresentCandidate,
) -> Result<Vec<SelectedOwnerGlyphCapture>, String> {
    with_dormant_evidence_automation(|automation| {
        automation.admit_selected_present(selected, selected.frame_hour)
    })
    .map(|selected| selected.unwrap_or_default())
}

pub fn reserve_selected_present_if_evidence_active(
    selected: OwnerGlyphPresentCandidate,
) -> Result<Option<SelectedOwnerGlyphCaptureReservation>, String> {
    with_dormant_evidence_automation(|automation| {
        automation.reserve_selected_present(selected, selected.frame_hour)
    })
}

pub fn reserve_selected_present_if_production_active(
    selected: OwnerGlyphPresentCandidate,
) -> Result<Option<SelectedOwnerGlyphCaptureReservation>, String> {
    with_production_automation(|automation| {
        automation.reserve_selected_present(selected, selected.frame_hour)
    })
}

pub fn commit_selected_present(reservation_id: u64) -> Result<(), String> {
    with_active_automation(|automation| automation.commit_selected_present(reservation_id))?
        .ok_or_else(|| "A25 owner-glyph automation is not installed".to_string())
}

pub fn rollback_selected_present(reservation_id: u64) -> Result<(), String> {
    with_active_automation(|automation| automation.rollback_selected_present(reservation_id))?
        .ok_or_else(|| "A25 owner-glyph automation is not installed".to_string())
}

pub fn admit_selected_present_if_production_active(
    selected: OwnerGlyphPresentCandidate,
) -> Result<Option<Vec<SelectedOwnerGlyphCapture>>, String> {
    with_production_automation(|automation| {
        automation.admit_selected_present(selected, selected.frame_hour)
    })
}
