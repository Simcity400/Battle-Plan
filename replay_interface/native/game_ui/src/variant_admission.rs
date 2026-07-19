//! XUI25 display/language/input variant admission.
//!
//! There is deliberately no nearest-resolution, nearest-scale, language
//! fallback, inferred font substitution, or reconstructed focus traversal in
//! this module. A replay selects one producer-authenticated variant envelope
//! by an exact key. File-derived layout laws can be used inside an envelope,
//! but a changed viewport, GUI scale, language, glyph set, or input mode is a
//! distinct envelope.

use crate::runtime_graphics::ClipRect;
use replay_core::world::DisplayVariantMetadata;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PixelExtent {
    pub width: u32,
    pub height: u32,
}

impl PixelExtent {
    fn valid(self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= i32::MAX as u32
            && self.height <= i32::MAX as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InputVariant {
    /// Current proven envelope: pointer-family dispatch. Authored keyboard
    /// shortcuts may invoke actions, but are not a focus-traversal graph.
    MouseKeyboard,
    /// Reserved until a producer-authenticated focus traversal exists.
    Keyboard,
    /// Reserved until a producer-authenticated focus traversal exists.
    Controller,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DisplayMode {
    Fullscreen,
    BorderlessFullscreen,
    Windowed,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DisplayVariantKey {
    pub physical_viewport: PixelExtent,
    pub min_gui: PixelExtent,
    /// Exact IEEE-754 bits read from the producing session. Using bits avoids
    /// decimal re-rounding and makes 1.0 distinct from any neighbouring value.
    pub gui_scale_f32_bits: u32,
    pub display_mode: DisplayMode,
    pub localisation_language: String,
    pub input_variant: InputVariant,
}

impl DisplayVariantKey {
    pub fn from_session_metadata(
        display: &DisplayVariantMetadata,
        localisation_language: &str,
        input_variant: InputVariant,
    ) -> Result<Self, VariantRejection> {
        if input_variant != InputVariant::MouseKeyboard {
            return Err(VariantRejection::UnsupportedInputVariant(input_variant));
        }
        let display_mode = match display.display_mode.as_str() {
            "fullscreen" => DisplayMode::Fullscreen,
            "borderless_fullscreen" => DisplayMode::BorderlessFullscreen,
            "windowed" => DisplayMode::Windowed,
            _ => {
                return Err(VariantRejection::InvalidEnvelope(
                    "unknown display mode".into(),
                ))
            }
        };
        let key = Self {
            physical_viewport: PixelExtent {
                width: display.physical_width,
                height: display.physical_height,
            },
            min_gui: PixelExtent {
                width: display.min_gui_width,
                height: display.min_gui_height,
            },
            gui_scale_f32_bits: display.gui_scale_f32_bits,
            display_mode,
            localisation_language: localisation_language.to_string(),
            input_variant,
        };
        if !key.valid() {
            return Err(VariantRejection::InvalidEnvelope(
                "invalid launch-bound display variant key".into(),
            ));
        }
        Ok(key)
    }

    pub fn gui_scale(self: &Self) -> f32 {
        f32::from_bits(self.gui_scale_f32_bits)
    }

    fn valid(&self) -> bool {
        let scale = self.gui_scale();
        self.physical_viewport.valid()
            && self.min_gui.valid()
            && scale.is_finite()
            && scale > 0.0
            && !self.localisation_language.trim().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontRunBinding {
    pub font_id: String,
    pub font_file_sha256: String,
    pub glyph_atlas_sha256: String,
    /// Hash of the exact ordered glyph-id/metrics/UV set consumed by this run.
    /// The atlas alone does not prove which glyph subset or ordering was used.
    pub glyph_set_sha256: String,
    pub first_glyph: u32,
    pub glyph_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedClip {
    pub element_id: String,
    /// Final half-open physical-pixel scissor after parent/viewport
    /// intersection. XUI07 owns the intersection law.
    pub clip: ClipRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusVariantState {
    pub active_window_id: String,
    pub focused_widget_id: String,
    /// Exact producer order for the selected input mode.
    pub traversal: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResizeTransition {
    pub previous_key: DisplayVariantKey,
    pub current_key: DisplayVariantKey,
    pub transition_ordinal: u64,
    pub post_resize_layout_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantEnvelope {
    pub source_record_id: String,
    pub evidence_manifest_sha256: String,
    pub key: DisplayVariantKey,
    /// Producer-resolved logical root size. No scale formula is inferred.
    pub logical_root: PixelExtent,
    /// Hash of the producer-resolved layout for this exact key.
    pub logical_layout_sha256: String,
    /// Hash of the admitted physical-pixel oracle. Envelopes without an exact
    /// pixel oracle are unsupported rather than borrowed from another key.
    pub pixel_oracle_sha256: String,
    pub font_runs: Vec<FontRunBinding>,
    pub clips: Vec<ResolvedClip>,
    pub focus: Option<FocusVariantState>,
    pub resize_transition: Option<ResizeTransition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariantRejection {
    InvalidEnvelope(String),
    DuplicateKey(DisplayVariantKey),
    UnsupportedExactVariant(DisplayVariantKey),
    UnsupportedInputVariant(InputVariant),
    ResizeTransitionRequired,
    ResizeTransitionMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantCatalog {
    envelopes: Vec<VariantEnvelope>,
}

impl VariantCatalog {
    pub fn admit(envelopes: Vec<VariantEnvelope>) -> Result<Self, VariantRejection> {
        let mut keys = BTreeSet::new();
        for envelope in &envelopes {
            validate_envelope(envelope)?;
            if !keys.insert(envelope.key.clone()) {
                return Err(VariantRejection::DuplicateKey(envelope.key.clone()));
            }
        }
        Ok(Self { envelopes })
    }

    /// Exact selection only. Unsupported aspect ratios, UI scales, languages,
    /// or input modes never borrow a neighbouring envelope.
    pub fn select(&self, key: &DisplayVariantKey) -> Result<&VariantEnvelope, VariantRejection> {
        self.envelopes
            .iter()
            .find(|row| &row.key == key)
            .ok_or_else(|| VariantRejection::UnsupportedExactVariant(key.clone()))
    }

    /// Select after an in-session variant change. A key change requires the
    /// producer-authenticated post-resize transition embedded in the target
    /// envelope; replay does not scale the previous layout forward.
    pub fn select_after_transition(
        &self,
        previous: &DisplayVariantKey,
        current: &DisplayVariantKey,
    ) -> Result<&VariantEnvelope, VariantRejection> {
        let selected = self.select(current)?;
        if previous == current {
            return Ok(selected);
        }
        let transition = selected
            .resize_transition
            .as_ref()
            .ok_or(VariantRejection::ResizeTransitionRequired)?;
        if &transition.previous_key != previous || &transition.current_key != current {
            return Err(VariantRejection::ResizeTransitionMismatch);
        }
        Ok(selected)
    }
}

fn validate_envelope(envelope: &VariantEnvelope) -> Result<(), VariantRejection> {
    let invalid = |message: &str| VariantRejection::InvalidEnvelope(message.to_string());
    if envelope.source_record_id.trim().is_empty() {
        return Err(invalid("missing source record id"));
    }
    if !is_sha256(&envelope.evidence_manifest_sha256) {
        return Err(invalid("invalid evidence manifest SHA-256"));
    }
    if !envelope.key.valid() || !envelope.logical_root.valid() {
        return Err(invalid(
            "invalid viewport, logical root, scale, or language",
        ));
    }
    if !is_sha256(&envelope.logical_layout_sha256) || !is_sha256(&envelope.pixel_oracle_sha256) {
        return Err(invalid("invalid layout or pixel-oracle SHA-256"));
    }
    if envelope.key.input_variant != InputVariant::MouseKeyboard {
        return Err(VariantRejection::UnsupportedInputVariant(
            envelope.key.input_variant,
        ));
    }
    if envelope.font_runs.is_empty() {
        return Err(invalid("no authenticated font/glyph runs"));
    }
    for run in &envelope.font_runs {
        if run.font_id.trim().is_empty()
            || !is_sha256(&run.font_file_sha256)
            || !is_sha256(&run.glyph_atlas_sha256)
            || !is_sha256(&run.glyph_set_sha256)
            || run.glyph_count == 0
            || run.first_glyph.checked_add(run.glyph_count).is_none()
        {
            return Err(invalid("invalid font/glyph run"));
        }
    }
    let viewport = ClipRect {
        left: 0,
        top: 0,
        right: envelope.key.physical_viewport.width as i32,
        bottom: envelope.key.physical_viewport.height as i32,
    };
    let mut clip_ids = BTreeSet::new();
    for row in &envelope.clips {
        if row.element_id.trim().is_empty()
            || !clip_ids.insert(row.element_id.clone())
            || !row.clip.is_nonempty()
            || row.clip.intersect(viewport) != Some(row.clip)
        {
            return Err(invalid(
                "invalid, duplicate, or out-of-viewport resolved clip",
            ));
        }
    }
    if envelope.focus.is_some() {
        return Err(invalid(
            "pointer-family variant must not invent traversal focus",
        ));
    }
    if let Some(transition) = &envelope.resize_transition {
        if transition.current_key != envelope.key
            || transition.previous_key == transition.current_key
            || transition.transition_ordinal == 0
            || !is_sha256(&transition.post_resize_layout_sha256)
            || transition.post_resize_layout_sha256 != envelope.logical_layout_sha256
        {
            return Err(invalid("invalid resize transition"));
        }
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
