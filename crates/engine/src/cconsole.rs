//! Console runtime output, excluded from hourly capture and content identity.
//! History paths select raw I/O sinks rather than content-stack files.

/// Constructor reading command history.
pub const CONSTRUCTOR_RVA: usize = 0x0206_ac60;
/// Accepted-command handler appending history.
pub const ACCEPT_COMMAND_RVA: usize = 0x0206_b710;
/// History filename literal storage.
pub const HISTORY_FILENAME_RVA: usize = 0x02ae_28e8;
/// History filename pointer slot.
pub const HISTORY_FILENAME_POINTER_RVA: usize = 0x0309_e568;
/// Constructor history-filename reference.
pub const HISTORY_READ_REFERENCE_RVA: usize = 0x0206_af4b;
/// Command-handler history-filename reference.
pub const HISTORY_WRITE_REFERENCE_RVA: usize = 0x0206_b82d;
/// History append-mode literal storage.
pub const HISTORY_APPEND_MODE_RVA: usize = 0x02ad_47e4;
/// History line-format literal storage.
pub const HISTORY_LINE_FORMAT_RVA: usize = 0x02ae_2ab8;
