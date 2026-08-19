pub const DATE_EPOCH_CONST: i32 = 43_799_999;

const MONTH_LENGTHS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn raw_date_in_range(stored: i32) -> bool {
    let min = (1850 * 365 * 24) + DATE_EPOCH_CONST;
    let max = (2200 * 365 * 24) + DATE_EPOCH_CONST;
    stored >= min && stored <= max
}

pub fn hour_index_from_raw(stored: i32) -> Option<i64> {
    if !raw_date_in_range(stored) {
        return None;
    }
    Some((stored - DATE_EPOCH_CONST) as i64)
}

pub fn date_from_raw(stored: i32) -> Option<String> {
    if !raw_date_in_range(stored) {
        return None;
    }
    let hrs = stored - DATE_EPOCH_CONST;
    let hour = hrs % 24;
    let rem_days = hrs / 24;
    let year = rem_days / 365;
    let mut doy = rem_days - year * 365;
    let mut month = 1;
    for len in MONTH_LENGTHS {
        if doy < len {
            break;
        }
        doy -= len;
        month += 1;
    }
    Some(format!("{year}.{month}.{}.{hour}", doy + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_index_matches_schema_definition() {
        let doy_index = 31 + 28 + 31 + 30 + 31 + 30 + 31 + 31 + 1;
        let hour_index = (1940i64 * 365 + doy_index) * 24 + 21;
        assert_eq!(hour_index, 17_000_277);
        let raw = DATE_EPOCH_CONST + hour_index as i32;
        assert_eq!(hour_index_from_raw(raw), Some(hour_index));
        assert_eq!(date_from_raw(raw).as_deref(), Some("1940.9.2.21"));
    }

    #[test]
    fn out_of_range_is_none() {
        assert_eq!(hour_index_from_raw(0), None);
        assert_eq!(date_from_raw(0), None);
    }

    #[test]
    fn roundtrip_hour_index_and_date_agree() {
        let raw = DATE_EPOCH_CONST + ((1936i64 * 365 + 10) * 24 + 5) as i32;
        let hi = hour_index_from_raw(raw).unwrap();
        assert_eq!(hi, (1936 * 365 + 10) * 24 + 5);
        assert_eq!(date_from_raw(raw).as_deref(), Some("1936.1.11.5"));
    }
}
