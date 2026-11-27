
use std::io;

pub const FIXED_SCALE: i64 = 100_000;

fn invalid(message: impl Into<String>) -> io::Error {
    crate::fields::invalid(message)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decimal {
    pub raw: i64,
    pub fractional_digits: u32,
    pub lexeme: Vec<u8>,
}

impl Decimal {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let (negative, unsigned) = match bytes.first() {
            Some(b'-') => (true, &bytes[1..]),
            Some(b'+') => (false, &bytes[1..]),
            _ => (false, bytes),
        };
        let mut parts = unsigned.split(|&byte| byte == b'.');
        let integer = parts.next().unwrap();
        let fraction = parts.next();
        if integer.is_empty()
            || parts.next().is_some()
            || !integer.iter().all(u8::is_ascii_digit)
            || fraction.is_some_and(|fraction| {
                fraction.is_empty() || !fraction.iter().all(u8::is_ascii_digit)
            })
        {
            return Err(invalid("save scalar is not a decimal number"));
        }
        let mut magnitude = 0_i128;
        for &digit in integer {
            magnitude = magnitude
                .checked_mul(10)
                .and_then(|n| n.checked_add(i128::from(digit - b'0')))
                .ok_or_else(|| invalid("save decimal exceeds integer representation"))?;
        }
        magnitude = magnitude
            .checked_mul(i128::from(FIXED_SCALE))
            .ok_or_else(|| invalid("save decimal exceeds integer representation"))?;
        let fraction = fraction.unwrap_or_default();
        if fraction
            .get(5..)
            .is_some_and(|tail| tail.iter().any(|&digit| digit != b'0'))
        {
            return Err(invalid(
                "save scalar has nonzero precision below one engine /100000 unit; refusing to round",
            ));
        }
        let mut place = 10_000_i128;
        for &digit in fraction.iter().take(5) {
            magnitude = magnitude
                .checked_add(i128::from(digit - b'0') * place)
                .ok_or_else(|| invalid("save decimal exceeds integer representation"))?;
            place /= 10;
        }
        let raw = i64::try_from(if negative { -magnitude } else { magnitude })
            .map_err(|_| invalid("save scalar exceeds the engine i64 representation"))?;
        Ok(Self {
            raw,
            fractional_digits: u32::try_from(fraction.len())
                .map_err(|_| invalid("decimal precision exceeds u32"))?,
            lexeme: bytes.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_preserve_engine_units_and_literal_precision_without_rounding() {
        for (literal, raw, digits) in [
            ("78.1411", 7_814_110, 4),
            ("0.7", 70_000, 1),
            ("-0.00001", -1, 5),
            ("1.000000", 100_000, 6),
            ("92233720368547.75807", i64::MAX, 5),
            ("-92233720368547.75808", i64::MIN, 5),
        ] {
            let value = Decimal::parse(literal.as_bytes()).unwrap();
            assert_eq!(value.raw, raw);
            assert_eq!(value.fractional_digits, digits);
            assert_eq!(value.lexeme, literal.as_bytes());
        }
        for bad in [
            "",
            ".1",
            "1.",
            "--1",
            "NaN",
            "1e2",
            "1.000001",
            "92233720368547.75808",
        ] {
            assert!(Decimal::parse(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}
