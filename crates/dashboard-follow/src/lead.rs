
pub const BYTES: usize = 128 << 20;
pub const LEAD: playhead::Lead = playhead::Lead { seconds: 0.5, bytes: BYTES };
pub const RELEASE_FRAMES: replay_data::Hour = 10;

pub fn budget() -> std::sync::Arc<playhead::ByteBudget> {
    static BUDGET: std::sync::OnceLock<std::sync::Arc<playhead::ByteBudget>> = std::sync::OnceLock::new();
    BUDGET.get_or_init(|| playhead::ByteBudget::new(BYTES)).clone()
}

static PRESSURE_MERGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn count_merge() { PRESSURE_MERGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
pub fn pressure_merges() -> u64 { PRESSURE_MERGES.load(std::sync::atomic::Ordering::Relaxed) }
