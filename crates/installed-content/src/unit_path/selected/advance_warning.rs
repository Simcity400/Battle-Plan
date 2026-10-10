//! Exact C684A0 arithmetic/text proposal. Thresholds are genuine native raw
//! operands, never hard-coded vanilla values. Formatter callback reuses the
//! recorded content/native numeric formatter at1424B6200; no `%` is invented.

use super::Fact;

/// C685CF..C68633: wrapped IMUL, signed /100000, unusual unsigned sentinel
/// guard, wrapped second IMUL, signed IDIV. EAX=FFFFFFFF zero-extends to i64.
pub fn organisation_warning_value(threshold: i64, maximum_organisation: i64) -> Fact<i64> {
    let adjusted = threshold.wrapping_mul(10_000_000) / 100_000;
    if maximum_organisation == 0
        || (adjusted as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc
    {
        return Ok(0xffff_ffff);
    }
    adjusted
        .wrapping_mul(100_000)
        .checked_div(maximum_organisation)
        .ok_or_else(|| "native low-organisation division would overflow".into())
}

/// Strength precedes organisation. Warning text REPLACES the prior buffer,
/// unlike common movement rejections. Max-org is demanded only for low-org.
pub fn can_advance(
    text: &mut String,
    strength: Fact<i64>,
    strength_threshold: Fact<i64>,
    mut organisation: impl FnMut() -> Fact<i64>,
    mut organisation_threshold: impl FnMut() -> Fact<i64>,
    mut maximum_organisation: impl FnMut() -> Fact<i64>,
    mut format_warning: impl FnMut(&str, i64) -> Fact<String>,
) -> Fact<bool> {
    let threshold = strength_threshold?;
    if strength? < threshold {
        *text = format_warning("NO_STRENGTH_TO_FIGHT", threshold)?;
        return Ok(false);
    }
    let threshold = organisation_threshold()?;
    if organisation()? >= threshold {
        return Ok(true);
    }
    let value = organisation_warning_value(threshold, maximum_organisation()?)?;
    *text = format_warning("NO_ORG_TO_FIGHT", value)?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_strength_replaces_and_skips_unknown_organisation() {
        let mut text = "earlier".to_string();
        assert_eq!(
            can_advance(
                &mut text,
                Ok(9),
                Ok(10),
                || panic!(),
                || Err("unreached".into()),
                || panic!(),
                |key, value| {
                    assert_eq!((key, value), ("NO_STRENGTH_TO_FIGHT", 10));
                    Ok("strength".into())
                }
            ),
            Ok(false)
        );
        assert_eq!(text, "strength");
    }
    #[test]
    fn org_limit_inclusive_skips_maximum_and_leaves_buffer_untouched() {
        let mut text = "earlier".to_string();
        assert_eq!(
            can_advance(
                &mut text,
                Ok(10),
                Ok(10),
                || Ok(20),
                || Ok(20),
                || panic!(),
                |_, _| panic!()
            ),
            Ok(true)
        );
        assert_eq!(text, "earlier");
    }
    #[test]
    fn raw_org_ratio_and_unsigned_sentinel_match_instructions() {
        assert_eq!(organisation_warning_value(100_000, 5_000_000), Ok(200_000));
        assert_eq!(organisation_warning_value(100_000, 0), Ok(4_294_967_295));
        assert_eq!(organisation_warning_value(-1, -2), Ok(5_000_000));
    }
    #[test]
    fn reached_maximum_refusal_does_not_invent_message_parameter() {
        let mut text = "earlier".to_string();
        assert!(
            can_advance(
                &mut text,
                Ok(10),
                Ok(10),
                || Ok(19),
                || Ok(20),
                || Err("maximum-org refused".into()),
                |_, _| panic!()
            )
            .is_err()
        );
        assert_eq!(text, "earlier");
    }
}
