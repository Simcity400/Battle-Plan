//! Agency upgrade definition identity. The branch reader constructs the
//! supported authored definition form. Acquired levels are stored separately;
//! a definition's level cap must not clamp stored acquisition values.

/// Upgrade-definition vtable.
pub const VTABLE_RVA: usize = 0x0285_cbc0;
/// Lexer token used by serialization.
pub const TOKEN: usize = 0x08;
/// Valid byte; loading accepts any nonzero value.
pub const VALID: usize = 0x10;
/// Owning branch pointer.
pub const BRANCH: usize = 0x260;
