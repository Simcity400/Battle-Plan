//! Shared exact-a729 engine text-hook substrate.
//!
//! The crate retains the authenticated textbox, final-setter, owner-glyph,
//! font-asset, inline-icon, and observer registries consumed by in-scope
//! extraction lanes. It does not define or emit a standalone compact domain.

pub mod owner_glyph_capture;

pub const MAX_TEXT_BYTES: usize = 1 << 20;

pub const TEXT_SETTER_RVA: u32 = 0x022B_59E0;
pub const TEXTBOX_REGISTRATION_RVA: u32 = 0x0224_5570;
pub const TEXTBOX_DESTRUCTOR_RVA: u32 = 0x0228_EC70;
/// Generic GUI sprite/image setter shared by registered observers.
pub const SPRITE_SETTER_RVA: u32 = 0x0229_A780;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeTextSourceIdentity {
    /// Exact raw `text = ...` token retained at `CTextBoxType+0x138`.
    AuthoredToken(String),
    /// No authored token was present; the exact setter caller is the source.
    SetterCallerRva(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeTextLayout {
    /// No owner-scoped render-layout observation has been admitted.
    AuthoredReproducible,
    /// An admitted owner-scoped observation supplied exact runs.
    CapturedRuntime {
        clip: Option<[i32; 4]>,
        glyphs: Vec<GlyphRun>,
        inline_icons: Vec<InlineIconRun>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphRun {
    /// Exact position in the mixed ordinary-glyph/inline-icon decision stream.
    pub run_ordinal: u32,
    pub font: String,
    pub glyph_id: u32,
    pub x: i32,
    pub y: i32,
    pub rgba: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineIconRun {
    /// Exact position in the mixed ordinary-glyph/inline-icon decision stream.
    pub run_ordinal: u32,
    pub sprite: String,
    pub frame: u32,
    pub x: i32,
    pub y: i32,
    pub baseline: i32,
    pub advance: i32,
}

/// Pointer-free observation shared between the hook and owner-glyph substrate.
///
/// This is process-local coordination state, not a compact-domain record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTextInput {
    pub context_id: String,
    pub pov_country_or_global: String,
    pub view_id: String,
    pub element_id: String,
    pub source: RuntimeTextSourceIdentity,
    pub final_text_bytes: Vec<u8>,
    pub setter_caller_rva: u32,
    pub formatter_observed: bool,
    pub layout: RuntimeTextLayout,
}

#[cfg(windows)]
pub mod a729_hook;
#[cfg(windows)]
pub mod a25_post_emit_hook;
#[cfg(windows)]
pub mod owner_glyph_probe;
