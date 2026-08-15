pub mod active_abilities;
pub mod alerts;
pub mod assets;
pub(crate) mod authenticated_semantic_provider;
pub mod authored_role_index;
pub mod binding;
pub mod character_ui;
pub mod combat_detail_canvas;
pub mod compose;
pub mod content_interpreter;
pub mod dds;
pub mod decision_chrome;
pub mod decision_observer;
pub mod decision_replay;
pub mod definition_admission;
pub mod design_catalog;
pub mod designer_observer;
pub mod engine_surface_roots;
pub mod engine_text_layout;
pub mod event_chrome;
pub mod focus_observer;
pub mod focus_observer_derivation;
pub mod font;
pub mod frontend_meta;
pub mod gfx;
pub mod gui;
pub(crate) mod gui_scene_util;
pub mod hud;
pub mod hud_role_index;
pub mod intelligence_presentation;
pub mod loc;
pub mod major_views;
pub mod markup;
pub mod mod_ui_discovery;
pub mod national_focus;
pub mod observer_collection;
pub mod observer_panels;
pub mod observer_technology;
pub mod occupation_binding;
pub mod occupation_compositor;
pub mod paradox_text;
pub mod peace_conference_ui;
pub mod png;
pub mod politics_state;
pub mod power_balance;
pub mod power_balance_compositor;
pub mod probe;
pub mod production_lines;
pub mod production_renderer;
pub mod readonly_design_peace;
pub mod research_canvas;
pub mod research_compositor;
pub mod research_observer;
pub mod research_state;
pub mod runtime_graphics;
pub mod scripted_gui_compositor;
pub mod scripted_gui_runtime;
pub mod scrollbar_law;
pub mod special_projects;
pub mod static_focus_defs;
pub mod static_research;
pub mod static_unit_catalogs;
pub mod text_law;
pub mod tga;
pub mod tooltip_compositor;
pub mod topbar;
pub mod trade_compositor;
pub mod trade_layout;
pub mod trade_state;
pub mod unitview;
pub mod variant_admission;
pub mod waroverview_binding;
pub mod widget_layout;
pub mod widget_visual_state;
#[cfg(test)]
mod widget_visual_state_tests;
pub mod window_manager_law;
pub mod world_tension;

pub use engine_text_layout::{
    engine_inline_icon_frame_index, layout_engine_text, layout_engine_text_with_icons,
    EngineTextBorder, EngineTextGlyph, EngineTextInlineIcon, EngineTextLayout,
    EngineTextLayoutError, EngineTextLine, ResolvedInlineIconMetrics,
};

use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, GameUiError>;

/// Shared hash-format law: exactly 64 lowercase ASCII hexadecimal digits, the
/// canonical rendering of a SHA-256 digest in this crate's manifests.
pub(crate) fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    Io,
    Parse,
    Missing,
    UnsupportedDdsFormat(String),
    UnsupportedTextureExtension(String),
    InvalidTexture(String),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameUiError {
    pub kind: ErrorKind,
    pub message: String,
}

impl GameUiError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn parse(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Parse, message)
    }

    pub fn missing(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Missing, message)
    }

    pub fn unsupported_dds(format: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ErrorKind::UnsupportedDdsFormat(format.into()), message)
    }

    pub fn unsupported_texture_extension(
        extension: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorKind::UnsupportedTextureExtension(extension.into()),
            message,
        )
    }
}

impl fmt::Display for GameUiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GameUiError {}

impl From<io::Error> for GameUiError {
    fn from(value: io::Error) -> Self {
        Self::new(ErrorKind::Io, value.to_string())
    }
}

impl From<String> for GameUiError {
    fn from(value: String) -> Self {
        Self::new(ErrorKind::Other, value)
    }
}

impl From<&str> for GameUiError {
    fn from(value: &str) -> Self {
        Self::new(ErrorKind::Other, value)
    }
}

impl From<portable_content_manifest::content_parse::ContentParseError> for GameUiError {
    fn from(value: portable_content_manifest::content_parse::ContentParseError) -> Self {
        Self::new(ErrorKind::Parse, value.message)
    }
}
