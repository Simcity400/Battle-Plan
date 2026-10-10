//! Combat-history retention policy. Pruning removes head entries older than
//! the inclusive lower retention bound and heads dated after the current hour.
//! Manager update prunes before resolving rounds, after the hourly date advance.
pub const RETENTION_HOURS: i32 = 0xa8;
