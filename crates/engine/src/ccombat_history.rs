//! CCombatHistory's engine-owned retention policy.
/// PruneHistory 140BA96C0 reads CGameState+0x468 and removes head entries
/// with entry+0x10 < now - 0xA8, and heads dated after now. The lower bound
/// is inclusive. Manager update 140BA9890 calls it on manager+0x28 before
/// resolving rounds; HourlyUpdate 1401DCEF0 has already advanced the date.
pub const RETENTION_HOURS: i32 = 0xa8;
