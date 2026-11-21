
pub const EPOCH: i32 = 43_800_000;
pub const HOURS_PER_YEAR: i32 = 8_760;
const DAYS_PER_YEAR: i32 = 365;
const MONTH_LENGTHS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
pub const PERIOD_DEFAULT_DATE: i32 = 43_808_760;
pub const PERIOD_DEFAULT_TEMPERATURE: [i64; 2] = [-1_000_000, 3_500_000];

pub fn normalize_to_year_one(date: i32) -> i32 {
    let years = date.wrapping_sub(EPOCH) / HOURS_PER_YEAR;
    date.wrapping_add(1_i32.wrapping_sub(years).wrapping_mul(HOURS_PER_YEAR))
}

fn days_before(month: i32) -> Result<i32, &'static str> {
    if month > 12 {
        return Err("weather period month past the native month table");
    }
    Ok(MONTH_LENGTHS[..month.max(0) as usize].iter().sum())
}

fn magnitude(value: i32) -> i32 {
    let negated = value.wrapping_neg();
    if negated < 0 { value } else { negated }
}

fn year_sign(years: i32) -> i32 {
    ((years >> 31) & -2).wrapping_add(1)
}

fn month_zero_based(date: i32) -> i32 {
    let day = (date.wrapping_sub(EPOCH) / 24) % DAYS_PER_YEAR;
    let mut month = 0;
    let mut rest = day.wrapping_sub(MONTH_LENGTHS[0]);
    while rest >= 0 && month < 11 {
        month += 1;
        rest = rest.wrapping_sub(MONTH_LENGTHS[month as usize]);
    }
    month
}

pub fn set_month_zero_based(date: i32, month: i32) -> Result<i32, &'static str> {
    let target = days_before(month)?;
    let days = date.wrapping_sub(EPOCH) / 24;
    let years = days / DAYS_PER_YEAR;
    let current = days_before(month_zero_based(date))?;
    let shifted = magnitude(years)
        .wrapping_sub(years)
        .wrapping_mul(DAYS_PER_YEAR)
        .wrapping_sub(current)
        .wrapping_add(days)
        .wrapping_add(target)
        .wrapping_mul(year_sign(years))
        .wrapping_sub(days);
    Ok(date.wrapping_add(shifted.wrapping_mul(24)))
}

pub fn set_day_of_month_zero_based(date: i32, day: i32) -> Result<i32, &'static str> {
    let hours = date.wrapping_sub(EPOCH);
    let days = hours / 24;
    let within = days
        .wrapping_add((hours / HOURS_PER_YEAR).wrapping_mul(-DAYS_PER_YEAR))
        .wrapping_sub(MONTH_LENGTHS[0]);
    let mut passed = 0;
    let mut rest = within;
    if rest >= 0 {
        loop {
            passed += 1;
            if passed >= MONTH_LENGTHS.len() {
                return Err("weather period day past the native month table");
            }
            rest = rest.wrapping_sub(MONTH_LENGTHS[passed]);
            if rest < 0 {
                break;
            }
        }
    }
    let start: i32 = MONTH_LENGTHS[..passed].iter().sum();
    let years = days / DAYS_PER_YEAR;
    let offset = magnitude(years)
        .wrapping_mul(DAYS_PER_YEAR)
        .wrapping_sub(days)
        .wrapping_add(start)
        .wrapping_add(day);
    Ok(hours
        .wrapping_add(offset.wrapping_mul(24))
        .wrapping_mul(year_sign(years))
        .wrapping_add(EPOCH))
}

pub fn apply_bound(date: i32, day: i32, month: i32) -> Result<i32, &'static str> {
    set_day_of_month_zero_based(
        set_month_zero_based(normalize_to_year_one(date), month)?,
        day,
    )
}

pub fn period_date(hour: i32) -> i32 {
    let years = (hour.wrapping_sub(EPOCH) / 24) / DAYS_PER_YEAR;
    let shifted = hour.wrapping_add(
        1_i32
            .wrapping_sub(years)
            .wrapping_mul(DAYS_PER_YEAR)
            .wrapping_sub(1_825_000)
            .wrapping_mul(24),
    );
    shifted.wrapping_sub(shifted % 24).wrapping_add(EPOCH)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub start: i32,
    pub end: i32,
    pub minimum: i64,
    pub maximum: i64,
}

pub fn find_period(periods: &[Period], hour: i32) -> Option<&Period> {
    let date = period_date(hour);
    periods
        .iter()
        .find(|period| period.start <= date && date <= period.end)
}

#[cfg(test)]
mod tests;
