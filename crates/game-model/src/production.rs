pub fn efficiency_average(sum: u64, count: i32) -> i64 {
    if count <= 0 {
        return 0;
    }
    ((sum << 15) as i64) / (i64::from(count) << 15)
}
