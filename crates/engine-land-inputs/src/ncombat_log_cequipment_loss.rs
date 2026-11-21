
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029d_9ef0);
pub const REASON: usize = 0x08;
pub const DATE: usize = 0x18;
pub const POOL: usize = 0x28;
pub const SIZE: usize = 0x68;

const EPOCH: i32 = 43_800_000;
const YEAR_HOURS: i32 = 8760;
const MONTH_DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

pub fn month_zero_based(date: i32) -> i32 {
    let mut day = (date.wrapping_sub(EPOCH) / 24) % 365 - MONTH_DAYS[0];
    let mut month = 0;
    while day >= 0 && month < 11 {
        month += 1;
        day -= MONTH_DAYS[month as usize];
    }
    month
}

pub fn month_bucket(date: i32) -> i32 {
    month_zero_based(date).wrapping_add((date.wrapping_sub(EPOCH) / YEAR_HOURS).wrapping_mul(12))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hour(year: i32, month: usize, day: i32, hour: i32) -> i32 {
        let days: i32 = MONTH_DAYS[..month - 1].iter().sum::<i32>() + day - 1;
        EPOCH + year * YEAR_HOURS + days * 24 + hour
    }

    #[test]
    fn month_bucket_follows_the_games_calendar() {
        assert_eq!(hour(1943, 11, 1, 18), 60_827_994);
        assert_eq!(month_zero_based(60_827_994), 10);
        assert_eq!(month_bucket(60_827_994), 1943 * 12 + 10);
        assert_eq!(month_bucket(hour(1943, 10, 31, 23)), 1943 * 12 + 9);
        assert_eq!(month_bucket(hour(1943, 11, 1, 0)), 1943 * 12 + 10);
        assert_eq!(month_bucket(hour(1943, 12, 31, 23)), 1943 * 12 + 11);
        assert_eq!(month_bucket(hour(1944, 1, 1, 0)), 1944 * 12);
        assert_eq!(month_bucket(hour(1942, 2, 14, 5)), 23_305);
    }
}
