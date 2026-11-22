#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_heap::Heap = engine_heap::Heap;


pub fn next_projection_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("projection identity space exhausted")
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub fn test_campaign() -> replay_frame::frame::SharedView {
        replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Campaign(vec![
            replay_data::rows::CampaignRow {
                id: b"synthetic".to_vec(),
            },
        ]))
        .into()
    }
}

pub mod actor_check;
pub mod actor_motion;
pub mod gradient;
pub mod lane;
pub mod line_lod;
pub mod mesh_budget;
pub mod mesh_effect;
pub mod mesh_shader;
pub mod particle_shader;
pub mod pointlights;
pub mod post;
pub mod projection_fold;
pub mod shader_constants;
pub mod shader_scene;
pub mod shader_sources;
#[cfg(test)]
mod shader_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod shader_validation;
pub mod shadow;
pub mod shadow_blur;
#[cfg(any(test, feature = "test-support"))]
pub mod test_game;
pub mod tree_caster;
