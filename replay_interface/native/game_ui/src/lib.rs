pub mod alerts;
pub mod assets;
pub mod binding;
pub mod binding_counters;
pub mod combat_binding;
pub mod compose;
pub mod dds;
pub mod event_chrome;
pub mod font;
pub mod gfx;
pub mod gui;
pub mod hud;
pub mod loc;
pub mod major_views;
pub mod markup;
pub mod paradox_text;
pub mod png;
pub mod probe;
pub mod tga;
pub mod topbar;
pub mod unitview;
pub mod waroverview_binding;
pub mod widget_layout;

use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, GameUiError>;

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
