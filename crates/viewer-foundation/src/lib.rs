//! Shared map GUI, GPU, geometry, visibility, engagement and worker contracts.

pub mod blocks;
pub mod combat_selection;
pub mod debug_fields;
pub mod diagnostics_panel;
pub mod engaged;
pub mod engagement_history;
pub mod engagement_progress;
pub mod errors;
pub mod icon_frames;
pub mod image_rgba;
pub mod land_combat_math;
pub mod map;
pub mod modifier_names;
pub mod movement_timing;
pub mod naval_check;
pub mod naval_icon_frames;
pub mod naval_labels;
pub mod naval_math;
pub mod naval_sides;
pub mod parallel;
pub mod profile_writer;
pub mod profiling;
pub mod resource_origins;
pub mod retirement;
pub mod state_resources;
pub mod table_access;
pub mod tooltip_painter;
pub mod tooltip_text;
pub mod trigger_content;
pub mod unit_handle;
pub mod worker_priority;

pub mod builder_pool;

pub mod unit_class;

pub mod recorded_labels;
