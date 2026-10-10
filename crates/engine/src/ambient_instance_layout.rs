//! Live instance for one ambient-object placement; its concrete type is unknown.
//! Countdown uses process seconds as f32, independently of game hours. This
//! presentation state is excluded from hourly recording.

/// Instance allocation size.
pub const BYTES: usize = 0x20;
/// Ambient-object placement pointer.
pub const OBJECT: usize = 0x00;
/// Render-entity pointer, cleared on destruction.
pub const ENTITY: usize = 0x08;
/// Expired byte, latched when duration reaches zero.
pub const EXPIRED: usize = 0x10;
/// Clock-unstarted byte, cleared on the first timer update.
pub const CLOCK_UNSTARTED: usize = 0x11;
/// Remaining duration, f32; initialized from the type and reduced by elapsed seconds.
pub const REMAINING_DURATION: usize = 0x14;
/// Previous process-clock value, f32.
pub const LAST_CLOCK: usize = 0x18;
/// Initializer joining the placement and type and constructing the render entity.
pub const INITIALIZE_RVA: usize = 0x0163_3d00;
/// Destructor clearing placement active state and incrementing its signed type counter when needed.
pub const DESTROY_RVA: usize = 0x0163_4070;
/// Timer consuming process-clock f32 to update countdown and expiry.
pub const TICK_RVA: usize = 0x0163_4900;
