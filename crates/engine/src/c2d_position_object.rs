//! Two-dimensional position object with graphical-object base fields, including
//! matrices, position, extents and clips. Static ownership alone does not prove
//! dynamic ancestry or lifetime.

/// Position-object vtable used by both embedded roots.
pub const VTABLE_RVA: usize = 0x02b2_6560;
/// Constructor building the graphical base, extents and scale-origin flag.
pub const CONSTRUCTOR_RVA: usize = 0x0222_0670;
