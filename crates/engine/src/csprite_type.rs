//! Ordinary sprite-type presentation layout. Initializer and resource
//! lifetime coverage is incomplete; no hourly walker is provided.

/// Allocation size.
pub const SIZE: usize = 0x1f0;
/// Factory constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0222_9de0;
/// Initialization entry setting frame direction and rows.
pub const INITIALIZE_RVA: usize = 0x0222_a020;
/// Authored field reader setting frame count and direction.
pub const READ_RVA: usize = 0x0222_c300;
/// Inherited initialization category, i32, assigned while loading the type.
pub const INITIALIZATION_CATEGORY: usize = 0x40;
/// Resource-initialization virtual-dispatch byte offset.
pub const INITIALIZE_RESOURCES_VSLOT: usize = 0x48;
/// Drawable-factory virtual-dispatch byte offset.
pub const CREATE_DRAWABLE_VSLOT: usize = 0xb8;
/// Size-query virtual-dispatch byte offset.
pub const SIZE_GETTER_VSLOT: usize = 0x88;
/// Size query also reaching the base texture loader.
pub const SIZE_GETTER_RVA: usize = 0x0222_b550;
/// Frame-dimension helper preceding integer size selection.
pub const FRAME_DIMENSIONS_RVA: usize = 0x0222_b4b0;
/// Base texture loader requesting zero skip, padding and sRGB flags.
pub const LOAD_BASE_TEXTURE_RVA: usize = 0x0222_b2a0;
/// Eager animation initializer that can clear the animation vector.
pub const INITIALIZE_ANIMATIONS_RVA: usize = 0x0222_b5f0;
/// Frame count, signed i32.
pub const FRAME_COUNT: usize = 0x90;
/// Frame-row count, signed i32, initialized to one.
pub const FRAME_ROWS: usize = 0x94;
/// Rotation, f32, initialized to zero.
pub const ROTATION: usize = 0xa0;
/// Texture width, signed i32, used for UV normalization.
pub const TEXTURE_WIDTH: usize = 0xa4;
/// Texture height, signed i32, used for UV normalization.
pub const TEXTURE_HEIGHT: usize = 0xa8;
/// Right crop, u32, copied from sprite-instance +0x190.
pub const CROP_RIGHT: usize = 0xac;
/// Bottom crop, u32, copied from sprite-instance +0x194.
pub const CROP_BOTTOM: usize = 0xb0;
/// Top crop, u32, copied from sprite-instance +0x19c.
pub const CROP_TOP: usize = 0xb4;
/// Left crop, u32, copied from sprite-instance +0x198.
pub const CROP_LEFT: usize = 0xb8;
/// Modulation-colour alpha, f32, multiplied by the per-instance opacity.
pub const MOD_COLOR_ALPHA: usize = 0xdc;
/// Nullable vertex-buffer pointer for four-vertex uploads.
pub const VERTEX_BUFFER: usize = 0xe0;
/// Geometry scale, f32, copied from world-matrix W.
pub const GEOMETRY_SCALE: usize = 0xf0;
/// Geometric stretch, f32, multiplying scaled vertex height.
pub const GEOMETRY_STRETCH: usize = 0xf4;
/// Texture V scale, f32, copied from sprite-instance +0x174.
pub const TEXTURE_V_SCALE: usize = 0xf8;
/// Zero-based selected frame, signed i32.
pub const SELECTED_FRAME: usize = 0xfc;
/// U frame step, f32, written before U flipping.
pub const FRAME_STEP_U: usize = 0x100;
/// V frame step, f32, written before U flipping.
pub const FRAME_STEP_V: usize = 0x104;
/// Frame direction, i32: authored boolean is widened, while evaluation handles 0, 1 and 2.
pub const FRAME_DIRECTION: usize = 0x10c;
/// U-flip byte copied from the sprite instance.
pub const FLIP_U: usize = 0x110;
/// Geometry-dirty byte selecting rebuild before drawing.
pub const GEOMETRY_DIRTY: usize = 0x108;
/// Animation-record vector pointer, stride 0x90.
pub const ANIMATIONS: usize = 0x118;
/// Animation-vector signed capacity checked on append.
pub const ANIMATION_CAPACITY: usize = 0x120;
/// Animation-vector signed count incremented on append.
pub const ANIMATION_COUNT: usize = 0x124;
/// Animation-vector allocator pointer used for growth.
pub const ANIMATION_ALLOCATOR: usize = 0x128;
/// Per-instance opacity, f32, multiplying modulation-colour alpha before upload.
pub const OPACITY: usize = 0x1a4;
/// X translation, f32, used only by backend kind zero.
pub const BACKEND_ZERO_TRANSLATION_X: usize = 0x1c8;
/// Y translation, f32, used only by backend kind zero.
pub const BACKEND_ZERO_TRANSLATION_Y: usize = 0x1cc;
/// Legacy lazy-load byte; constructor-global initial value unresolved.
pub const LEGACY_LAZY_LOAD: usize = 0x1d3;
/// Right/bottom crop setter copying instance operands.
pub const SET_CROP_RIGHT_BOTTOM_RVA: usize = 0x0222_dc10;
/// Left/top crop setter copying instance operands.
pub const SET_CROP_LEFT_TOP_RVA: usize = 0x0222_dee0;
/// Texture V-scale setter.
pub const SET_TEXTURE_V_SCALE_RVA: usize = 0x0222_dc50;
/// U-flip setter.
pub const SET_FLIP_U_RVA: usize = 0x0222_dc20;
/// UV and vertex builder using crops, frame geometry and scales.
pub const BUILD_GEOMETRY_RVA: usize = 0x0222_cb90;
/// Geometry-rebuild dispatch byte offset used for dirty or changed scale.
pub const BUILD_GEOMETRY_VSLOT: usize = 0xf0;
/// Frame-offset selector preceding signed frame conversion.
pub const FRAME_OFFSET_RVA: usize = 0x0222_a990;
/// Effect-program selector overriding default only for effect kinds 2, 3 and 4.
pub const SELECT_EFFECT_PROGRAM_RVA: usize = 0x0222_d1a0;
/// Draw and constant-upload entry.
pub const DRAW_RVA: usize = 0x0222_d5a0;
/// Start of the CPU matrix loop using separate multiplication and addition.
pub const MATRIX_UPLOAD_LOOP_START_RVA: usize = 0x0222_d7b0;
/// Final CPU matrix-loop store before drawing.
pub const MATRIX_UPLOAD_LOOP_STORE_RVA: usize = 0x0222_d7e8;
