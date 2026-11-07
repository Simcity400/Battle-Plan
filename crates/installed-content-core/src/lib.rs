//! Game content layers, identity hashes, and Rust parsers for the viewer.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod adjacencies;
pub mod adjacency_rules;
pub mod ambient;
pub mod autonomy;
pub mod buildings;
pub mod characters;
pub mod cities;
pub mod collections;
pub mod combat_tactics;
pub mod content;
pub mod country_scorers;
pub mod country_tags;
pub mod coverage;
pub mod crt;
pub mod decisions;
pub mod defines;
pub mod diplomacy;
pub mod doctrines;
pub mod dynamic_modifiers;
pub mod effect_keys;
pub mod equipment;
pub mod faction_goals;
pub mod faction_templates;
pub mod faction_upgrades;
#[doc(hidden)]
pub mod fields;
pub mod fixed5;
pub mod flags;
pub mod gui;
pub mod ideas;
pub mod ideologies;
pub use installed_content_image as image;
pub mod industrial_organisations;
pub mod intelligence;
pub mod key_names;
pub mod leader_traits;
pub mod map;
pub mod mesh;
pub mod mio_traits;
pub mod modifier_definitions;
pub mod modifiers;
pub mod movement;
pub mod names;
pub mod occupation_laws;
pub mod occupation_modifiers;
pub mod opinion_modifiers;
pub mod parallel;
pub mod parser;
pub mod particles;
pub mod pdxshader;
pub mod positions;
pub mod power_balance;
pub mod province_geometry;
pub mod province_graph;
pub mod railways;
pub mod resources;
pub mod scientist_traits;
pub mod scope;
pub mod scoped_variable;
pub mod script_constants;
pub mod script_files;
pub mod scripted_effects;
pub mod scripted_keys;
pub mod scripted_map_modes;
pub mod scripted_triggers;
pub mod special_projects;
pub mod static_modifiers;
pub mod strategic_locations;
pub mod tag_aliases;
pub mod technology;
pub mod technology_sharing;
pub mod terrain;
#[cfg(any(test, feature = "layer-fixtures"))]
pub mod test_support;
pub mod tokens;
pub mod train_gfx;
pub mod trigger_keys;
pub mod triggers;
pub mod unit_leader;
pub mod unit_medals;
pub mod variable_builtins;

pub use content::{
    ChangedLayer, GatherOptions, GatheredContent, HashStats, LayerRole, ResolveOptions,
    ResolvedContent, ResolvedLayer, ResolvedLayers, gather, gather_cancellable, playback, resolve,
    resolve_cancellable,
};

/// Severity assigned where a content reader creates a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticKind {
    /// Text the tree parser cannot parse.
    ParseFailure,
    /// The native reader reports a field and continues.
    Misread,
    /// Reading stops before the rest of a file can be determined.
    Unsettled,
    /// The port does not model this form.
    Unsupported,
    /// A definition or lookup cannot be used.
    Refused,
    /// Several definitions leave the intended result uncertain.
    Ambiguous,
    /// Presentation substitutes for unavailable content.
    Fallback,
    /// The game logs an error and continues.
    Logged,
    /// Another content note that does not prevent its use.
    Warning,
}

/// A diagnostic's producer-assigned kind and unchanged message.
pub type Diagnostic = (DiagnosticKind, String);

/// How a consumer takes a producer's diagnostic.
pub trait DiagnosticMessage: Sized {
    fn from_diagnostic(note: Diagnostic) -> Self;
}

impl DiagnosticMessage for Diagnostic {
    fn from_diagnostic(note: Diagnostic) -> Self {
        note
    }
}

impl DiagnosticMessage for String {
    fn from_diagnostic((_, message): Diagnostic) -> Self {
        message
    }
}
