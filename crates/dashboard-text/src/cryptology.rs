
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Defines {
    pub base: i64,
    pub upgrade: i64,
}

impl Defines {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> Result<Self, String> {
        let read = |name| {
            defines
                .fixed5("NOperatives", name)
                .map_err(|e| e.to_string())
        };
        Ok(Self {
            base: read("CRYPTO_BASE_CRYPTO_LEVEL")?,
            upgrade: read("CRYPTO_CRYPTO_LEVEL_PER_CRYPTO_UPGRADE")?,
        })
    }
}

pub fn threshold(strength: i64, defines: Defines) -> i64 {
    (i64::from(crate::triggers::fixed_round(strength) as i32)
        .wrapping_mul(100_000)
        .wrapping_mul(defines.upgrade)
        / 100_000)
        .wrapping_add(defines.base)
}

pub fn fully_decrypted(amount: i64, active: u8, threshold: Option<i64>) -> Option<bool> {
    if active != 0 {
        Some(true)
    } else {
        threshold.map(|required| amount >= required)
    }
}

pub fn progress_percent(amount: i64, active: u8, threshold: i64) -> Option<u8> {
    let ratio = if threshold == 0
        || (amount as u64).wrapping_add(0x7fff_ffff_ffff_fffe) > 0xffff_ffff_ffff_fffc
    {
        0xffff_ffff
    } else {
        amount.wrapping_mul(100_000).checked_div(threshold)?
    };
    let ratio = if active != 0 { 100_000 } else { ratio };
    Some((crate::triggers::fixed_round(ratio.wrapping_mul(100)) as i32).clamp(1, 100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_rounds_narrows_and_bonus_short_circuits() {
        let d = Defines {
            base: 200_000,
            upgrade: 300_000,
        };
        assert_eq!(threshold(149_999, d), 500_000);
        assert_eq!(threshold(150_000, d), 800_000);
        assert_eq!(threshold(-150_000, d), -400_000);
        assert_eq!(fully_decrypted(799_999, 0, Some(800_000)), Some(false));
        assert_eq!(fully_decrypted(800_000, 0, Some(800_000)), Some(true));
        assert_eq!(fully_decrypted(-1, 7, None), Some(true));
        assert_eq!(fully_decrypted(0, 0, None), None);
        assert_eq!(
            threshold(
                214_748_364_800_000,
                Defines {
                    base: 0,
                    upgrade: 1
                }
            ),
            -2_147_483_648
        );
    }

    #[test]
    fn decryption_meter_has_a_minimum_frame_and_uses_wrapping_fixed5() {
        let values: Vec<(String, u8, String, Option<u8>)> =
            serde_json::from_str(include_str!("cryptology/test-values.json")).unwrap();
        for (amount, active, threshold, expected) in values {
            assert_eq!(
                progress_percent(amount.parse().unwrap(), active, threshold.parse().unwrap()),
                expected,
                "{amount}/{threshold}, active {active}"
            );
        }
    }
}
