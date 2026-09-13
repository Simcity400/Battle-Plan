//! The game-identical map pipeline (`RENDERING.md`). CPU-side generators live beside the passes
//! that consume them; each module owns one intermediate or one pass.

// borders
pub mod border_pass;
pub mod borders;
pub mod camera;
pub mod frame;
pub mod gradient;
// names
pub mod labels;
pub mod name_pass;
pub mod names;
// political fill
pub mod political;
// colour
pub mod post;
pub mod scene;
pub mod sprites;
pub mod terrain;
pub mod text;
pub mod textures;
pub mod water;
