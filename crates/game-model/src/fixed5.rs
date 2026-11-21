
pub const SCALE: i64 = 100_000;

pub fn fraction(numerator: i64, denominator: i64) -> Result<i64, &'static str> {
    if denominator == 0
        || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc
    {
        return Ok(0xffff_ffff);
    }
    numerator
        .wrapping_mul(SCALE)
        .checked_div(denominator)
        .ok_or("native fixed5 fraction traps")
}

pub fn log(mut value: i64) -> Result<i64, &'static str> {
    if value <= 0 || value == SCALE {
        return Ok(0);
    }
    let mut exponent = 0_i32;
    while value.wrapping_abs() >= 200_000 {
        value = fraction(value, 271_828)?;
        exponent = exponent.wrapping_add(1);
    }
    let reduced = value.wrapping_sub(SCALE);
    let mut power = reduced;
    let mut sum = reduced;
    let mut divisor = 1_i32;
    loop {
        divisor = divisor.wrapping_add(1);
        power = power.wrapping_mul(reduced) / -SCALE;
        let term = fraction(power, i64::from(divisor) * SCALE)?;
        sum = sum.wrapping_add(term);
        if term == 0 {
            return Ok((i64::from(exponent) * SCALE).wrapping_add(sum));
        }
    }
}

pub fn exp(value: i64) -> Result<i64, &'static str> {
    let mut term = SCALE;
    let mut sum = SCALE;
    let mut divisor = 1_i32;
    loop {
        let factor = fraction(value, i64::from(divisor) * SCALE)?;
        divisor = divisor.wrapping_add(1);
        term = factor.wrapping_mul(term) / SCALE;
        sum = sum.wrapping_add(term);
        if term == 0 {
            return Ok(sum);
        }
    }
}

pub fn power(base: i64, exponent: i64) -> Result<i64, &'static str> {
    let negate = base < 0 && exponent & 1 != 0;
    let logarithm = log(if base < 0 { base.wrapping_neg() } else { base })?;
    let value = exp(logarithm.wrapping_mul(exponent) / SCALE)?;
    Ok(if negate { value.wrapping_neg() } else { value })
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_exponential_can_converge_after_sixty_five_thousand_terms() {
        assert_eq!(super::exp(6_553_600_000), Ok(46_758_153_478_750_530));
    }
}
