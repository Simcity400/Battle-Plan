use content_reader_defines::defines::Defines;
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivisionExperience {
    thresholds: Vec<i64>,
    base_level: i32,
    combat_factor: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DivisionExperienceProgress {
    pub level: i32,
    pub next_level_fraction: i64,
    pub combat_bonus: i64,
}

impl DivisionExperience {
    pub fn read(defines: &Defines) -> io::Result<Self> {
        let thresholds = defines.fixed5_array("NMilitary", "UNIT_EXP_LEVELS")?;
        if thresholds.is_empty() {
            return Err(content_reader_parser::fields::invalid(
                "NMilitary.UNIT_EXP_LEVELS is empty",
            ));
        }
        Ok(Self {
            thresholds,
            base_level: defines.int32("NMilitary", "ARMY_EXP_BASE_LEVEL")?,
            combat_factor: defines.fixed5("NMilitary", "EXPERIENCE_COMBAT_FACTOR")?,
        })
    }

    pub fn progress(&self, experience: i64) -> DivisionExperienceProgress {
        let level = self
            .thresholds
            .iter()
            .take_while(|&&threshold| threshold <= experience)
            .count();
        let previous = level.checked_sub(1).map_or(0, |at| self.thresholds[at]);
        let upper = self.thresholds.get(level).copied().unwrap_or(100_000);
        let numerator = experience.wrapping_sub(previous);
        let denominator = upper.wrapping_sub(previous);
        let sentinel =
            (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd;
        let fraction = if denominator == 0 || sentinel {
            0xffff_ffff
        } else {
            numerator
                .wrapping_mul(100_000)
                .checked_div(denominator)
                .unwrap_or(0xffff_ffff)
        };
        DivisionExperienceProgress {
            level: level as i32,
            next_level_fraction: if level == self.thresholds.len() {
                if denominator == 0 || sentinel {
                    100_000
                } else {
                    fraction.clamp(0, 100_000)
                }
            } else {
                fraction
            },
            combat_bonus: ((level as i32).wrapping_sub(self.base_level) as i64)
                .wrapping_mul(100_000)
                .wrapping_mul(self.combat_factor)
                / 100_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn division_experience_uses_mod_threshold_order_boundaries_and_final_progress() {
        let defines = Defines::new().unwrap();
        defines.load("synthetic.lua", b"NDefines={NMilitary={UNIT_EXP_LEVELS={0.2,0.2,0.15,0.8},ARMY_EXP_BASE_LEVEL=1,EXPERIENCE_COMBAT_FACTOR=0.4}}").unwrap();
        let content = DivisionExperience::read(&defines).unwrap();
        assert_eq!(
            content.progress(19_999),
            DivisionExperienceProgress {
                level: 0,
                next_level_fraction: 99_995,
                combat_bonus: -40_000
            }
        );
        assert_eq!(
            content.progress(20_000),
            DivisionExperienceProgress {
                level: 3,
                next_level_fraction: 7_692,
                combat_bonus: 80_000
            }
        );
        assert_eq!(content.progress(80_000).next_level_fraction, 0);
        assert_eq!(content.progress(90_000).next_level_fraction, 50_000);
        assert_eq!(content.progress(200_000).next_level_fraction, 100_000);
        assert_eq!(content.progress(-10_000).next_level_fraction, -50_000);
    }
}
