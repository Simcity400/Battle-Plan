//! The game-identical map pipeline ([archived rendering notes](#)). CPU-side generators live beside the passes
//! that consume them; each module owns one intermediate or one pass.

// counters
pub mod aircraft;
pub mod allegiance;
pub mod atlas_pages;
// borders
pub mod border_pass;
pub mod borders;
pub mod camera;
// counters
pub mod counter_eta;
pub mod counter_groups;
pub mod counter_inputs;
pub mod counter_move;
pub mod counter_order;
pub mod counter_portraits;
pub mod counter_status;
pub mod counters;
pub mod country_index;
// borders: the demilitarised-zone bucket bank
pub mod demilitarized;
// counters: the one ETA source, recorded or derived
pub mod division_eta;
pub mod entity_animation;
pub mod entity_effects;
pub mod frame;
pub mod gpu_passes;
pub mod gradient;
// names
// close zoom: the shared static-mesh pass and the layers built on it
pub mod map_buildings;
pub mod memo;
pub mod models;
pub mod railways;
pub mod trains;
// names
pub mod labels;
pub mod layers;
pub mod lazy_icons;
// the map's table frame
pub mod mapframe;
pub mod name_pass;
pub mod names;
// counters: the viewing country and what it can see
pub mod perspective;
// orders: the recorded player's battle plan
pub mod orders;
// political fill
pub mod pointlights;
pub mod political;
pub mod political_gpu;
pub mod raids;
pub mod railway_gun_counter;
// colour
pub mod post;
// rivers
pub mod rivers;
pub mod scene;
// shadows: the light's depth map, the screen-space pre-pass and the blurs
pub mod shadow;
pub mod sprawl;
pub mod sprites;
/// What each hour's tables hold, and what a gap in one costs the render.
pub mod tables;
pub mod templates;
pub mod terrain;
pub mod text;
pub mod textures;
// trees
pub mod trees;
// the screen-space interface's scale
pub mod ui_scale;
// close zoom: the 3D land-model selectors and their inputs
pub mod camouflage;
pub mod equipment_graphics;
pub mod model_countries;
pub mod model_multi_fallback;
pub mod model_selection;
pub mod unit_models;
pub mod visibility;
pub mod water;
// weather
pub mod weather;
pub mod weather_effects;

#[cfg(test)]
mod shader_tests;
