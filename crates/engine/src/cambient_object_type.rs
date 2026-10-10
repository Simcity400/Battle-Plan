//! Authored ambient definition shared by placements. Activation count is signed
//! i32 and duration is f32. No presentation lifecycle is installed here.

/// Definition allocation size.
pub const BYTES: usize = 0xa8;
/// Animation string.
pub const ANIMATION: usize = 0x28;
/// Use-animation byte, initially one.
pub const USE_ANIMATION: usize = 0x48;
/// Scale, f32, initially one.
pub const SCALE: usize = 0x4c;
/// Duration, f32, initially zero and copied to instance countdown.
pub const DURATION: usize = 0x50;
/// Maximum-visible count, i32, initially -1; comparisons remain signed.
pub const MAX_VISIBLE: usize = 0x78;
/// Always-visible byte, initially zero, used by the reap gate.
pub const ALWAYS_VISIBLE: usize = 0x7c;
/// Spawn counter, i32; activation decrements with a zero clamp, removal increments.
pub const SPAWN_COUNTER: usize = 0x80;
/// DLC-condition trigger pointer.
pub const DLC_TRIGGER: usize = 0x88;
/// DLC trigger shared-ownership control block.
pub const DLC_TRIGGER_CONTROL: usize = 0x90;
/// Constructor installing vtables and defaults.
pub const CONSTRUCTOR_RVA: usize = 0x0163_3fe0;
/// Keyword dispatcher for animation, duration and placement.
pub const KEYWORD_DISPATCHER_RVA: usize = 0x0163_4490;
/// Keyword id parsing the use-animation byte.
pub const USE_ANIMATION_KEYWORD_ID: u32 = 0x16e;
/// Keyword id allocating a placement and assigning its type.
pub const AMBIENT_OBJECT_KEYWORD_ID: u32 = 0x41;
