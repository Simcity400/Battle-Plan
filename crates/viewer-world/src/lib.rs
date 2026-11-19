//! Native world modules with no host or sibling feature dependency.

/// Tests run on the allocator the viewer ships with.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

pub mod map {
    pub(crate) fn next_projection_id() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("projection identity space exhausted")
    }

    /// Folds keep their state only while the campaign table stays one allocation.
    #[cfg(test)]
    pub(crate) fn test_campaign() -> replay_frame::frame::SharedView {
        replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Campaign(vec![
            replay_data::rows::CampaignRow {
                id: b"synthetic".to_vec(),
            },
        ]))
        .into()
    }

    pub mod actor_check;
    pub(crate) mod actor_motion;
    pub mod adjacency_icons;
    pub mod air_assignment;
    pub mod air_orders;
    pub mod air_ranges;
    pub mod air_transfers;
    pub mod aircraft;
    pub mod aircraft_events;
    pub mod animated_map_text;
    pub mod arrow_shader;
    pub mod border_pass;
    pub mod border_shader;
    pub mod borders;
    pub mod camouflage;
    pub mod demilitarized;
    pub mod entity_animation;
    pub mod entity_effects;
    pub mod equipment_graphics;
    pub mod gradient;
    pub mod labels;
    pub mod map_buildings;
    pub mod map_mode;
    pub mod mapframe;
    pub mod mesh_budget;
    pub mod mesh_effect;
    pub mod mesh_pose;
    pub mod mesh_shader;
    pub mod model_labels;
    pub mod model_multi_fallback;
    pub mod model_selection;
    pub mod models;
    pub mod movement_arrows;
    pub mod name_pass;
    pub mod name_shader;
    pub mod names;
    pub mod naval_dominance;
    pub mod naval_selection;
    pub mod occupation_icons;
    pub mod occupation_stripes;
    pub mod order_pass;
    pub mod particle_shader;
    pub mod pointlights;
    pub mod political;
    pub mod political_gpu;
    pub(crate) mod poses;
    pub mod post;
    pub mod projection_fold;
    pub mod radar_symbols;
    pub mod rail_shader;
    pub mod railways;
    pub mod resource_icons;
    pub mod river_shader;
    pub mod rivers;
    pub mod scene;
    pub mod scene_content;
    #[cfg(any(test, feature = "test-support"))]
    pub mod scene_faults;
    pub mod shader_constants;
    pub mod shader_scene;
    pub mod shader_sources;
    #[cfg(test)]
    mod shader_tests;
    pub mod shadow;
    pub mod shadow_blur;
    pub mod sky_shader;
    pub mod sound_events;
    pub mod sprawl;
    pub mod strait_shader;
    pub mod strike_effects;
    pub mod terrain;
    pub mod terrain_shader;
    pub mod trade_routes;
    pub mod trains;
    pub mod tree_caster;
    pub mod tree_order;
    pub mod tree_receiver;
    pub mod tree_shader;
    pub mod trees;
    pub mod unit_history;
    pub mod unit_models;
    pub mod water;
    pub mod water_shader;
    pub mod weather;
    pub mod weather_effects;
}
