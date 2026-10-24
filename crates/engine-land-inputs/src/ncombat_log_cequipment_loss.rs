//! One calendar month of an orders group's equipment losses or recoveries.
//!
//! Construction stores the reason, current date and copied equipment pool.
//! The loader initially uses reason four. Writers add quantities to the last
//! entry only when its month matches the current month; otherwise they append.
//! The stored date therefore remains the date of its first write. Serialization
//! includes the equipment pool, reason and date.

/// Equipment-loss primary vtable.
pub const VTABLE_RVA: usize = 0x0295_d568;
/// Stored signed i32 reason; loaded values above three are dropped.
pub const REASON: usize = 0x08;
/// First write's raw game hour.
pub const DATE: usize = 0x18;
/// Embedded equipment pool with fixed5 quantities.
pub const POOL: usize = 0x28;
/// Equipment-loss allocation size.
pub const SIZE: usize = 0x68;

/// Raw game hour of the CGameDate epoch (the date arithmetic subtracts it).
const EPOCH: i32 = 43_800_000;
/// Hours in the game's fixed 365-day year.
const YEAR_HOURS: i32 = 8760;
/// Loaded month-length table.
const MONTH_DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Zero-based month for a raw game hour using the fixed 365-day calendar and i32 arithmetic.
pub fn month_zero_based(date: i32) -> i32 {
    let mut day = (date.wrapping_sub(EPOCH) / 24) % 365 - MONTH_DAYS[0];
    let mut month = 0;
    while day >= 0 && month < 11 {
        month += 1;
        day -= MONTH_DAYS[month as usize];
    }
    month
}

/// Month bucket combining the zero-based month with the elapsed fixed-calendar years.
pub fn month_bucket(date: i32) -> i32 {
    month_zero_based(date).wrapping_add((date.wrapping_sub(EPOCH) / YEAR_HOURS).wrapping_mul(12))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raw game hour of a date as the game counts it (365-day years, hour 0..23).
    fn hour(year: i32, month: usize, day: i32, hour: i32) -> i32 {
        let days: i32 = MONTH_DAYS[..month - 1].iter().sum::<i32>() + day - 1;
        EPOCH + year * YEAR_HOURS + days * 24 + hour
    }

    #[test]
    fn month_bucket_follows_the_games_calendar() {
        // Calendar fixture for hour 19, with text-save hour numbering starting at one.
        assert_eq!(hour(1943, 11, 1, 18), 60_827_994);
        assert_eq!(month_zero_based(60_827_994), 10);
        assert_eq!(month_bucket(60_827_994), 1943 * 12 + 10);
        // The last hour of October and the first of November differ by one.
        assert_eq!(month_bucket(hour(1943, 10, 31, 23)), 1943 * 12 + 9);
        assert_eq!(month_bucket(hour(1943, 11, 1, 0)), 1943 * 12 + 10);
        assert_eq!(month_bucket(hour(1943, 12, 31, 23)), 1943 * 12 + 11);
        assert_eq!(month_bucket(hour(1944, 1, 1, 0)), 1944 * 12);
        // A February fixture uses the same month-bucket scheme.
        assert_eq!(month_bucket(hour(1942, 2, 14, 5)), 23_305);
    }
}
