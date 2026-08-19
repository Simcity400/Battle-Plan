
use crate::is_lowercase_sha256;
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
    MouseKeyboard,
    Keyboard,
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
    pub gui_scale_f32_bits: u32,
    pub display_mode: DisplayMode,
    pub localisation_language: String,
    pub input_variant: InputVariant,
}

impl DisplayVariantKey {
    pub fn from_playback_environment(
        environment: &map_viewer::PlaybackEnvironment,
        input_variant: InputVariant,
    ) -> Result<Self, VariantRejection> {
        if input_variant != InputVariant::MouseKeyboard {
            return Err(VariantRejection::UnsupportedInputVariant(input_variant));
        }
        let setting = |name: &str| {
            environment.settings.get(name).ok_or_else(|| {
                VariantRejection::InvalidEnvelope(format!(
                    "local playback profile lacks {name}"
                ))
            })
        };
        let parse_u32 = |name: &str| -> Result<u32, VariantRejection> {
            setting(name)?.parse::<u32>().map_err(|_| {
                VariantRejection::InvalidEnvelope(format!(
                    "local playback profile {name} is not a u32"
                ))
            })
        };
        let display_mode = match setting("display_mode")?.as_str() {
            "fullscreen" => DisplayMode::Fullscreen,
            "borderless_fullscreen" => DisplayMode::BorderlessFullscreen,
            "windowed" => DisplayMode::Windowed,
            _ => {
                return Err(VariantRejection::InvalidEnvelope(
                    "local playback profile has an unknown display mode".into(),
                ))
            }
        };
        let key = Self {
            physical_viewport: PixelExtent {
                width: parse_u32("physical_width")?,
                height: parse_u32("physical_height")?,
            },
            min_gui: PixelExtent {
                width: parse_u32("min_gui_width")?,
                height: parse_u32("min_gui_height")?,
            },
            gui_scale_f32_bits: parse_u32("gui_scale_f32_bits")?,
            display_mode,
            localisation_language: environment.localisation_language.clone(),
            input_variant,
        };
        if !key.valid() {
            return Err(VariantRejection::InvalidEnvelope(
                "invalid local playback UI profile".into(),
            ));
        }
        Ok(key)
    }

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
pub struct PlaybackUiProfile {
    key: DisplayVariantKey,
    settings_file_sha256: String,
}

impl PlaybackUiProfile {
    pub fn admit(
        environment: &map_viewer::PlaybackEnvironment,
        actual_client_input: InputVariant,
    ) -> Result<Self, VariantRejection> {
        let settings_file_sha256 = environment
            .settings
            .get("settings_file_sha256")
            .filter(|value| is_lowercase_sha256(value))
            .ok_or_else(|| {
                VariantRejection::InvalidEnvelope(
                    "local playback profile lacks a valid settings_file_sha256".into(),
                )
            })?
            .clone();
        Ok(Self {
            key: DisplayVariantKey::from_playback_environment(
                environment,
                actual_client_input,
            )?,
            settings_file_sha256,
        })
    }

    pub fn key(&self) -> &DisplayVariantKey {
        &self.key
    }

    pub fn physical_viewport(&self) -> PixelExtent {
        self.key.physical_viewport
    }

    pub fn localisation_language(&self) -> &str {
        &self.key.localisation_language
    }

    pub fn actual_client_input(&self) -> InputVariant {
        self.key.input_variant
    }

    pub fn settings_file_sha256(&self) -> &str {
        &self.settings_file_sha256
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontRunBinding {
    pub font_id: String,
    pub font_file_sha256: String,
    pub glyph_atlas_sha256: String,
    pub glyph_set_sha256: String,
    pub first_glyph: u32,
    pub glyph_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedClip {
    pub element_id: String,
    pub clip: ClipRect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusVariantState {
    pub active_window_id: String,
    pub focused_widget_id: String,
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
    pub logical_root: PixelExtent,
    pub logical_layout_sha256: String,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactKeyVariantContract {
    admitted: DisplayVariantKey,
}

impl ExactKeyVariantContract {
    pub fn admit(admitted: DisplayVariantKey) -> Result<Self, VariantRejection> {
        if !admitted.valid() {
            return Err(VariantRejection::InvalidEnvelope(
                "invalid mechanically narrow display variant key".into(),
            ));
        }
        if admitted.input_variant != InputVariant::MouseKeyboard {
            return Err(VariantRejection::UnsupportedInputVariant(
                admitted.input_variant,
            ));
        }
        Ok(Self { admitted })
    }

    pub fn select(
        &self,
        requested: &DisplayVariantKey,
    ) -> Result<&DisplayVariantKey, VariantRejection> {
        if requested != &self.admitted {
            return Err(VariantRejection::UnsupportedExactVariant(
                requested.clone(),
            ));
        }
        Ok(&self.admitted)
    }
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

    pub fn select(&self, key: &DisplayVariantKey) -> Result<&VariantEnvelope, VariantRejection> {
        self.envelopes
            .iter()
            .find(|row| &row.key == key)
            .ok_or_else(|| VariantRejection::UnsupportedExactVariant(key.clone()))
    }

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
    if !is_lowercase_sha256(&envelope.evidence_manifest_sha256) {
        return Err(invalid("invalid evidence manifest SHA-256"));
    }
    if !envelope.key.valid() || !envelope.logical_root.valid() {
        return Err(invalid(
            "invalid viewport, logical root, scale, or language",
        ));
    }
    if !is_lowercase_sha256(&envelope.logical_layout_sha256)
        || !is_lowercase_sha256(&envelope.pixel_oracle_sha256)
    {
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
            || !is_lowercase_sha256(&run.font_file_sha256)
            || !is_lowercase_sha256(&run.glyph_atlas_sha256)
            || !is_lowercase_sha256(&run.glyph_set_sha256)
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
            || !is_lowercase_sha256(&transition.post_resize_layout_sha256)
            || transition.post_resize_layout_sha256 != envelope.logical_layout_sha256
        {
            return Err(invalid("invalid resize transition"));
        }
    }
    Ok(())
}
