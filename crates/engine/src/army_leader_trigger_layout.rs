//! Army-leader predicate trigger; its concrete type is unknown.

/// Boolean operand compared with whether the resolved leader category is below two.
pub const OPERAND: usize = 0x58;
/// Evaluator resolving a character and then its leader; the receiver is the trigger.
pub const EVALUATE_RVA: usize = 0x0044_0db0;
