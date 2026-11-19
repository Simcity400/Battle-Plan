//! Common map inputs, resources and rendering contracts.

pub mod allegiance;
pub mod anchored;
pub mod atlas_pages;
pub mod overlay_count;
pub mod camera;
pub mod count_math;
pub mod country_index;
pub mod device_shared;
pub mod directed_wars;
pub mod frame;
pub mod frame_budget;
pub mod frame_head;
pub mod frame_limit;
pub mod hour_check;
pub mod publication_check;
pub mod free_icons;
pub mod gpu_contract;
pub mod display_refresh;
pub mod gpu_frame_time;
pub mod gpu_passes;
pub mod gradient_math;
pub mod gui_resources;
pub mod icon_windows;
pub mod layers;
pub mod lazy_icons;
pub mod memo;
pub mod mode_visibility;
pub mod msvc_stable_sort;
pub mod native_effects;
pub mod native_flags;
pub mod native_gui;
pub mod native_resources;
pub mod native_shader;
pub mod native_text;
pub mod native_windows;
pub mod pass_key;
pub mod perspective;
pub mod presentation;
pub mod province_admission;
pub mod province_geometry;
pub mod province_map;
pub mod province_pick;
pub mod province_schedule;
pub mod redraws;
pub mod simulation_rate;
pub mod sprites;
pub mod stack_anchors;
pub mod tables;
pub mod templates;
pub mod terrain_content;
pub mod terrain_sight;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod text;
pub mod textures;
pub mod ui_scale;
pub mod visibility;

pub mod air_detection;
pub mod air_range;
pub mod air_region_reach;
pub mod aircraft_model;

pub mod model_admission;

pub mod movement_reads;

pub mod building_facts;

pub mod color_inheritance;

pub mod subunit_files;

pub mod label_reads;

pub mod movement_routes;

pub mod ribbon_geometry;

pub mod unit_rules;

pub mod air_participants;
pub mod air_reachability;
pub mod air_registry_sources;
pub mod aircraft_count;
pub mod counter_move;
pub mod counter_order;
pub mod task_force_visual;

pub mod unit_pose;

pub mod name_draw;

pub mod ribbon_content;
pub mod ribbon_mesh;

pub mod ribbon_fixed;
pub mod ribbon_native;

pub mod air_transport_content;

pub mod counter_action;
pub mod counter_inputs;
pub mod counter_status;
pub mod naval_count_approx;
pub mod naval_dominance_sources;
pub mod railway_gun_counter;

pub mod counter_attrition;
pub mod port_mission_defines;

pub mod counter_anchors;
pub mod counter_hovers;

pub mod border_power;

pub mod division_panel;

pub mod naval_mine_levels;

pub mod naval_region_control;
