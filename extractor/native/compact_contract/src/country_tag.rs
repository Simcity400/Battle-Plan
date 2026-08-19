//!   against the literal `3`. In a release build the assert is compiled out but

/// length equals this, and the release build silently skips any key of another
pub const ENGINE_COUNTRY_TAG_LEN: usize = 3;

pub const NO_COUNTRY_SENTINEL: &str = "---";

pub fn is_engine_country_tag(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == ENGINE_COUNTRY_TAG_LEN
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

pub fn is_engine_country_tag_or_no_country_sentinel(value: &str) -> bool {
    value == NO_COUNTRY_SENTINEL || is_engine_country_tag(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_tags_are_ordinary_country_tags() {
        for tag in ["D01", "D04", "D42", "D75"] {
            assert!(
                is_engine_country_tag(tag),
                "{tag} is a vanilla dynamic country tag and must be admitted"
            );
            assert!(is_engine_country_tag_or_no_country_sentinel(tag));
        }
    }

    #[test]
    fn admission_does_not_depend_on_a_d_prefix() {
        for tag in ["X01", "CW3", "Z9Z", "0AB", "999", "A1B"] {
            assert!(
                is_engine_country_tag(tag),
                "{tag} is a three-byte [A-Z0-9] tag and must be admitted without a prefix rule"
            );
        }
    }

    #[test]
    fn static_tags_stay_admitted() {
        for tag in ["GER", "ENG", "SOV", "FRA", "USA"] {
            assert!(is_engine_country_tag(tag));
        }
    }

    #[test]
    fn only_the_engine_tag_length_is_admitted() {
        for value in ["", "G", "GE", "GERM", "GERMANY", "D0", "D045"] {
            assert!(
                !is_engine_country_tag(value),
                "{value:?} is not {ENGINE_COUNTRY_TAG_LEN} bytes and must be refused"
            );
            assert!(!is_engine_country_tag_or_no_country_sentinel(value));
        }
    }

    #[test]
    fn misread_shapes_are_refused() {
        for value in [
            "ger",
            "Ge1",
            "G R",
            "G\0R",
            "G-R",
            "G_R",
            "G.R",
            "\u{fffd}er",
            "  G",
        ] {
            assert!(
                !is_engine_country_tag(value),
                "{value:?} is not a plausible engine tag and must be refused"
            );
        }
    }

    #[test]
    fn the_no_country_sentinel_is_only_admitted_by_the_sentinel_predicate() {
        assert!(!is_engine_country_tag(NO_COUNTRY_SENTINEL));
        assert!(is_engine_country_tag_or_no_country_sentinel(
            NO_COUNTRY_SENTINEL
        ));
        for value in ["-", "--", "----", "-G-", "GE-"] {
            assert!(!is_engine_country_tag(value));
            assert!(!is_engine_country_tag_or_no_country_sentinel(value));
        }
    }

    #[test]
    fn the_rule_is_exactly_three_bytes_of_upper_alnum_everywhere() {
        for a in 0u8..128 {
            for b in 0u8..128 {
                for c in 0u8..128 {
                    let bytes = [a, b, c];
                    let Ok(value) = core::str::from_utf8(&bytes) else {
                        continue;
                    };
                    let expected = bytes
                        .iter()
                        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit());
                    assert_eq!(
                        is_engine_country_tag(value),
                        expected,
                        "three-byte tag {bytes:?} does not match the stated rule"
                    );
                    assert_eq!(
                        is_engine_country_tag_or_no_country_sentinel(value),
                        expected || value == NO_COUNTRY_SENTINEL,
                        "three-byte tag {bytes:?} does not match the stated sentinel rule"
                    );
                }
            }
        }
    }

    #[test]
    fn the_cid1_roster_tag_width_is_the_engine_tag_length() {
        assert_eq!(
            crate::campaign_identity::CID1_ROSTER_TAG_LEN,
            ENGINE_COUNTRY_TAG_LEN
        );
    }
}
