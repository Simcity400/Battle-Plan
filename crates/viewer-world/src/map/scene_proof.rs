//! Test-only observations from the real Scene; no mock presentation or allocator.
use super::*;
use std::collections::BTreeSet;

/// Same-frame semantic observations, including a fatal attempt's reached partial state.
/// These do not replace the App's earlier warning snapshot or consume its audio reset.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneReports {
    pub generation: u64,
    pub arena_frame: u64,
    pub pass_diagnostics: passes::Diagnostics,
    pub draw_refusals: BTreeSet<String>,
    pub shader_refusals: BTreeMap<String, u64>,
    pub warnings: Vec<String>,
    pub sound_events: Vec<crate::map::sound_events::SoundEvent>,
    pub sound_refusals: BTreeMap<String, usize>,
    pub model_draws: Vec<u32>,
}
impl SceneReports {
    pub(super) fn capture(scene: &Scene) -> Self {
        Self {
            generation: scene.generation,
            arena_frame: scene.arena.frame(),
            pass_diagnostics: scene.pass_diagnostics.clone(),
            draw_refusals: scene.draw_refusals(),
            shader_refusals: scene.shader_refusals().clone(),
            warnings: scene.pass_warnings(),
            sound_events: scene.sound_events.clone(),
            sound_refusals: scene.sound_refusals.clone(),
            model_draws: model_passes!(scene)
                .into_iter()
                .flatten()
                .map(crate::map::models::ModelPass::drawn)
                .collect(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct SceneProof {
    pub reports: SceneReports,
    /// Canonical model layer owners for comparisons between independent Scenes.
    /// The reports above keep their actual, unmodified event keys.
    pub sound_owner_keys: Vec<u64>,
    pub arena: Vec<u8>,
    pub sound_bits: Vec<([u32; 3], u32)>,
    pub last_animation_time: Option<u32>,
    pub grading: String,
    pub shadows: bool,
    pub grid: ([u32; 2], u32, [u32; 2]),
    pub targets: Option<([u32; 2], [u32; 2])>,
    pub overlays: (u32, u32),
    pub submissions: u64,
    pub attempted: Vec<passes::Pass>,
    pub reached: Vec<crate::map::scene_faults::Point>,
    pub timer_present: bool,
    pub native_batches: Vec<(usize, Option<usize>)>,
}
impl Scene {
    pub fn frame_proof_for_tests(&self) -> SceneProof {
        let grid = self.grid();
        let mut owner_keys = BTreeMap::new();
        for (layer, model) in model_passes!(self).into_iter().enumerate() {
            if let Some(model) = model {
                for event in &model.events {
                    let actual = crate::map::sound_events::key(&event.key);
                    let mut canonical = event.key.clone();
                    canonical.pass = layer as u64 + 1;
                    owner_keys.insert(actual, crate::map::sound_events::key(&canonical));
                }
            }
        }
        SceneProof {
            reports: SceneReports::capture(self),
            sound_owner_keys: self
                .sound_events
                .iter()
                .map(|event| owner_keys.get(&event.key).copied().unwrap_or(event.key))
                .collect(),
            arena: self.arena.proof_bytes_for_tests(),
            sound_bits: self
                .sound_events
                .iter()
                .map(|e| (e.position.map(f32::to_bits), e.age.to_bits()))
                .collect(),
            last_animation_time: self.last_animation_time.map(f32::to_bits),
            grading: format!("{:?}", self.post.last_grading()),
            shadows: self.shadows(),
            grid: (
                grid.origin.map(f32::to_bits),
                grid.step.to_bits(),
                grid.counts,
            ),
            targets: self.targets().map(|t| (t.size(), t.world_size())),
            overlays: self.overlay_counts(),
            submissions: self.test_faults.submissions.get(),
            attempted: self.test_faults.attempted.borrow().clone(),
            reached: self.test_faults.reached.borrow().clone(),
            timer_present: self.pass_timer.is_some(),
            native_batches: self.test_faults.native_batches.borrow().clone(),
        }
    }
}
