//! The game-identical map pipeline (`RENDERING.md`). CPU-side generators live beside the passes
//! that consume them; each module owns one intermediate or one pass.

// borders
pub mod border_pass;
pub mod borders;
pub mod camera;
pub mod frame;
pub mod gradient;
pub mod names;
// political fill
pub mod political;
pub mod scene;
pub mod terrain;
pub mod textures;
pub mod water;
