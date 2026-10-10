//! GUI application-timer construction and resume call chain.
//! These call identities do not establish a clock-state walker.

/// GUI application-timer origin caller.
pub const APPLICATION_TIMER_ORIGIN_CALLER_RVA: usize = 0x0234_0d80;
/// GUI application-timer origin callee.
pub const APPLICATION_TIMER_ORIGIN_CALLEE_RVA: usize = 0x0223_8d50;
