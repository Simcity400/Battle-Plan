//! Host map composition and producers that combine multiple feature outputs.

#[cfg(test)]
mod offline_scene;

// Removed when the in-flight hours tests import foundation directly.
#[cfg(test)]
pub use viewer_foundation::map::camera;
#[cfg(test)]
pub use viewer_foundation::map::test_support;

// Removed when the in-flight hour tests import their selection owner.
#[cfg(test)]
pub mod unit_selection {
    pub use viewer_foundation::unit_handle::Unit;
}

#[cfg(test)]
mod player_model_tests;
#[cfg(test)]
pub mod scene_failure_tests;
#[cfg(test)]
mod scene_render_tests;
