/// Average of `count` entries with wrapping u64 `sum`. Nonpositive counts
/// return zero. Otherwise both operands are shifted left by 15 bits and
/// divided as signed i64 values, truncating toward zero. The caller uses the
/// minimum of active and maximum factories.
pub fn efficiency_average(sum: u64, count: i32) -> i64 {
    if count <= 0 {
        return 0;
    }
    ((sum << 15) as i64) / (i64::from(count) << 15)
}
