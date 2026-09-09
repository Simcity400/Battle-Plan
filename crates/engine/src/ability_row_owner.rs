//! Command-ability GUI row owner; exact engine class name unproven.
//!
//! Source: `extractor/native/lanes/active_command_abilities_live/src/a729_hook.rs`.
//! This object is reached through the selected controller's row-widget vector.

/// Row's embedded hovered widget; read_tooltip_identity matches owner+0x30 to the tooltip argument (source:1909-1918).
pub const WIDGET: usize = 0x30;
