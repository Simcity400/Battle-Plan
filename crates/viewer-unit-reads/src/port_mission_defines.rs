use crate::counter_inputs::SCALE;
use std::io;

#[derive(Clone, Debug)]
pub struct ScoreDefines {
    pub screening_ratio: Result<i64, String>,
    pub efficiency: [i64; 6],
}

#[derive(Clone, Debug)]
pub struct Defines {
    pub score: Result<ScoreDefines, String>,
    pub estimate_width: [Result<i32, String>; 2],
    pub likelihood: [Result<i64, String>; 3],
}

impl Defines {
    pub fn read(d: &content_reader_defines::defines::Defines) -> Self {
        let score = (|| {
            let keys = [
                "AGGRESSION_HEAVY_GUN_EFFICIENCY_ON_HEAVY_SHIPS",
                "AGGRESSION_HEAVY_GUN_EFFICIENCY_ON_LIGHT_SHIPS",
                "AGGRESSION_LIGHT_GUN_EFFICIENCY_ON_HEAVY_SHIPS",
                "AGGRESSION_LIGHT_GUN_EFFICIENCY_ON_LIGHT_SHIPS",
                "AGGRESSION_TORPEDO_EFFICIENCY_ON_HEAVY_SHIPS",
                "AGGRESSION_TORPEDO_EFFICIENCY_ON_LIGHT_SHIPS",
            ];
            let mut efficiency = [0; 6];
            for (out, key) in efficiency.iter_mut().zip(keys) {
                *out = d.fixed5("NNavy", key)?;
            }
            Ok::<_, io::Error>(ScoreDefines {
                screening_ratio: d
                    .fixed5("NNavy", "SCREEN_RATIO_FOR_FULL_SCREENING_FOR_CAPITALS")
                    .map(|value| value.max(1_000))
                    .map_err(|e| e.to_string()),
                efficiency,
            })
        })()
        .map_err(|e| e.to_string());
        Self {
            score,
            estimate_width: [
                "INTEL_LEVEL_LOW_STRENGTH_ESTIMATE_HALF_RANGE_PERCENTAGE",
                "INTEL_LEVEL_MEDIUM_STRENGTH_ESTIMATE_HALF_RANGE_PERCENTAGE",
            ]
            .map(|key| {
                d.int32("NNavy", key)
                    .map(|v| v.clamp(0, 100))
                    .map_err(|e| e.to_string())
            }),
            likelihood: [
                "NAVAL_STRIKE_FORCE_ATTACK_LIKELYHOOD_THR_VERY_LIKELY",
                "NAVAL_STRIKE_FORCE_ATTACK_LIKELYHOOD_THR_LIKELY",
                "NAVAL_STRIKE_FORCE_ATTACK_LIKELYHOOD_THR_UNLIKELY",
            ]
            .map(|key| {
                d.fixed5("NInterface", key)
                    .map(|v| v.clamp(0, SCALE))
                    .map_err(|e| e.to_string())
            }),
        }
    }
}
