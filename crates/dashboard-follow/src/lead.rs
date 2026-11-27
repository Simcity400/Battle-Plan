
pub const BYTES: usize = 128 << 20;
pub const LEAD: playhead::Lead = playhead::Lead { seconds: 0.5, bytes: BYTES / 2 };
pub const RELEASE_FRAMES: replay_data::Hour = 10;

pub fn budget() -> std::sync::Arc<playhead::ByteBudget> {
    static BUDGET: std::sync::OnceLock<std::sync::Arc<playhead::ByteBudget>> = std::sync::OnceLock::new();
    BUDGET.get_or_init(|| playhead::ByteBudget::new(BYTES)).clone()
}

pub fn export_budget() -> std::sync::Arc<playhead::ByteBudget> {
    static EXPORT_BUDGET: std::sync::OnceLock<std::sync::Arc<playhead::ByteBudget>> = std::sync::OnceLock::new();
    EXPORT_BUDGET.get_or_init(|| playhead::ByteBudget::new(BYTES)).clone()
}

static DROPPED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn count_dropped(count: u64) { DROPPED.fetch_add(count, std::sync::atomic::Ordering::Relaxed); }
pub fn dropped() -> u64 { DROPPED.load(std::sync::atomic::Ordering::Relaxed) }
