
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
}

impl Date {
    pub fn parse(s: &str) -> Option<Date> {
        let s = s.trim();
        let mut parts = s.split('.');
        let year = parts.next()?.trim().parse::<i64>().ok()?;
        let month = parts.next()?.trim().parse::<u32>().ok()?;
        let day = parts.next()?.trim().parse::<u32>().ok()?;
        let hour = match parts.next() {
            Some(h) => h.trim().parse::<u32>().ok()?,
            None => 0,
        };
        if parts.next().is_some() {
            return None;
        }
        if month == 0 || month > 12 || day == 0 || day > 31 || hour > 23 {
            return None;
        }
        Some(Date {
            year,
            month,
            day,
            hour,
        })
    }

    pub fn day_key(&self) -> (i64, u32, u32) {
        (self.year, self.month, self.day)
    }

    pub fn day_label(&self) -> String {
        format!("{}.{}.{}", self.year, self.month, self.day)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_and_without_hour() {
        assert_eq!(
            Date::parse("1936.1.1.12"),
            Some(Date {
                year: 1936,
                month: 1,
                day: 1,
                hour: 12
            })
        );
        assert_eq!(
            Date::parse("1941.10.15"),
            Some(Date {
                year: 1941,
                month: 10,
                day: 15,
                hour: 0
            })
        );
    }

    #[test]
    fn rejects_malformed() {
        assert_eq!(Date::parse("hour 12"), None);
        assert_eq!(Date::parse("1936.13.1"), None);
        assert_eq!(Date::parse("1936.1.0"), None);
        assert_eq!(Date::parse("1936.1.1.24"), None);
        assert_eq!(Date::parse("1936.1.1.0.5"), None);
        assert_eq!(Date::parse("1936.1"), None);
        assert_eq!(Date::parse(""), None);
    }

    #[test]
    fn chronological_ordering() {
        let a = Date::parse("1936.1.1.0").unwrap();
        let b = Date::parse("1936.1.1.12").unwrap();
        let c = Date::parse("1936.1.2.0").unwrap();
        let d = Date::parse("1937.1.1.0").unwrap();
        assert!(a < b && b < c && c < d);
        assert_eq!(a, Date::parse("1936.1.1").unwrap());
    }

    #[test]
    fn day_key_and_label() {
        let d = Date::parse("1936.3.7.9").unwrap();
        assert_eq!(d.day_key(), (1936, 3, 7));
        assert_eq!(d.day_label(), "1936.3.7");
    }
}
