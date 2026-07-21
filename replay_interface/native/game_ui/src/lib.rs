pub mod active_abilities;
pub mod alerts;
pub mod army_orders_group;
pub mod assets;
pub mod binding;
pub mod binding_counters;
pub mod character_ui;
pub mod combat_binding;
pub mod combat_detail_losses;
pub mod compose;
pub mod content_interpreter;
pub mod dds;
pub mod decision_chrome;
pub mod decision_replay;
pub mod definition_admission;
pub mod design_catalog;
pub mod dynamic_collection;
pub mod event_chrome;
pub mod font;
pub mod frontend_meta;
pub mod gfx;
pub mod gui;
pub mod gui_runtime_law;
pub(crate) mod gui_scene_util;
pub mod hud;
pub mod hud_role_index;
pub mod intelligence_ui;
pub mod loc;
pub mod major_views;
pub mod markup;
pub mod mod_ui_discovery;
pub mod national_focus;
pub mod occupation_binding;
pub mod paradox_text;
pub mod politics_tooltips;
pub mod preferred_tactic;
pub mod png;
pub mod power_balance;
pub mod power_balance_assets;
pub mod probe;
pub mod production_renderer;
pub mod readonly_design_peace;
pub mod research_canvas;
pub mod research_state;
pub mod runtime_graphics;
pub mod scripted_gui_runtime;
pub mod scrollbar_law;
pub mod selected_present;
pub mod special_projects;
pub mod text_law;
pub mod tga;
pub mod tooltip_compositor;
pub mod topbar;
pub mod trade_state;
pub mod unitview;
pub mod variant_admission;
pub mod waroverview_binding;
pub mod widget_layout;
pub mod widget_visual_state;
pub mod window_manager_law;
pub mod world_tension;

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
