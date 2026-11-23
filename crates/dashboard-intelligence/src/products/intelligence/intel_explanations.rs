use super::intel_details::IntelValue;
use content_reader_defines::defines::Defines;
use replay_data::rows::IntelContributionRow;

fn value(name: impl Into<String>, value: impl ToString) -> IntelValue {
    IntelValue {
        name: name.into(),
        value: value.to_string(),
    }
}
pub(super) fn country_caps(defines: &Defines) -> Result<[i64; 4], String> {
    let values = defines
        .fixed5_array("NIntel", "COUNTRY_LEVEL_INTEL_MAXIMUMS")
        .map_err(|e| e.to_string())?;
    Ok(std::array::from_fn(|lane| {
        values.get(lane).copied().unwrap_or(100_000)
    }))
}
fn source_caps(defines: &Defines, name: &str) -> Result<[i64; 4], String> {
    let values = defines
        .fixed5_array("NIntel", name)
        .map_err(|e| e.to_string())?;
    if values.len() < 4 {
        return country_caps(defines);
    }
    Ok(values[..4].try_into().unwrap())
}
const DYNAMIC: [&str; 6] = [
    "EVENT",
    "LAND_COMBAT",
    "NAVAL_COMBAT",
    "AIR_COMBAT",
    "AIR_RECON",
    "CAPTURED_OPERATIVE",
];
const STATIC: [&str; 4] = [
    "OPERATION_TOKENS",
    "BROKEN_CYPHER",
    "RADAR",
    "INTEL_NETWORK",
];
pub(super) fn source(
    defines: &Defines,
    row: &IntelContributionRow,
) -> Result<Vec<IntelValue>, String> {
    let mut out = Vec::new();
    let country = country_caps(defines)?;
    out.push(value(
        "Country caps (civilian / army / navy / air), fixed5",
        format!("{country:?}"),
    ));
    if row.kind == 2 {
        let caps = match row.pool {
            0 | 5 => country,
            1..=4 => source_caps(
                defines,
                &format!(
                    "STATIC_INTEL_SOURCE_{}_MAXIMUMS",
                    STATIC[row.pool as usize - 1]
                ),
            )?,
            _ => return Err(format!("unknown reached static intel pool {}", row.pool)),
        };
        out.push(value("Source caps, fixed5", format!("{caps:?}")));
    } else if matches!(row.kind, 3..=6) {
        let Some(index) = row.pool.checked_sub(1) else {
            out.push(value("Pool", "Neutral pool; flat decay 0, multiplier 1"));
            return Ok(out);
        };
        let name = *DYNAMIC
            .get(index as usize)
            .ok_or_else(|| format!("unknown reached dynamic intel pool {}", row.pool))?;
        out.push(value("Source", name));
        for suffix in ["MAXIMUMS", "ABSOLUTE_MAXIMUMS"] {
            let key = format!("DYNAMIC_INTEL_SOURCE_{name}_{suffix}");
            let cap = if suffix == "ABSOLUTE_MAXIMUMS" {
                let values = defines
                    .fixed5_array("NIntel", &key)
                    .map_err(|e| e.to_string())?;
                const ACCUMULATOR_CEILING: i64 = 92233720200000;
                if values.len() < 4 {
                    [ACCUMULATOR_CEILING; 4]
                } else {
                    values[..4].try_into().unwrap()
                }
            } else {
                source_caps(defines, &key)?
            };
            out.push(value(format!("{suffix}, fixed5"), format!("{cap:?}")));
        }
        let flat = defines
            .fixed5("NIntel", &format!("DYNAMIC_INTEL_SOURCE_{name}_FLAT_DECAY"))
            .map_err(|e| e.to_string())?
            .max(0);
        let factor = defines
            .fixed5("NIntel", &format!("DYNAMIC_INTEL_SOURCE_{name}_MULT_DECAY"))
            .map_err(|e| e.to_string())?
            .max(0);
        out.push(value("Daily flat decay, fixed5", flat));
        out.push(value("Daily multiplier, fixed5", factor));
        if row.kind == 6 && row.ordinal != u32::MAX {
            let mut pool = engine_intelligence::intel_sources::DynamicPool {
                accumulators: vec![engine_intelligence::intel_sources::Lanes {
                    target: row.target,
                    values: [row.civilian, row.army, row.navy, row.air],
                }],
                ..Default::default()
            };
            engine_intelligence::intel_sources::decay_accumulators(&mut pool, flat, factor);
            out.push(value(
                "After one decay of these operands, fixed5",
                format!("{:?}", pool.accumulators[0].values),
            ));
        }
    }
    if row.ordinal != u32::MAX && matches!(row.kind, 2 | 3 | 6) {
        let caps = if row.kind == 2 {
            match row.pool {
                0 | 5 => country,
                1..=4 => source_caps(
                    defines,
                    &format!(
                        "STATIC_INTEL_SOURCE_{}_MAXIMUMS",
                        STATIC[row.pool as usize - 1]
                    ),
                )?,
                _ => return Err(format!("unknown reached static intel pool {}", row.pool)),
            }
        } else if row.pool == 0 {
            country
        } else {
            let name = DYNAMIC
                .get(row.pool as usize - 1)
                .ok_or_else(|| format!("unknown reached dynamic intel pool {}", row.pool))?;
            source_caps(defines, &format!("DYNAMIC_INTEL_SOURCE_{name}_MAXIMUMS"))?
        };
        for ((name, raw), cap) in ["Civilian", "Army", "Navy", "Air"]
            .into_iter()
            .zip([row.civilian, row.army, row.navy, row.air])
            .zip(caps)
        {
            out.push(value(
                format!("{name} contribution cap"),
                if raw > cap {
                    "Recorded operand exceeds contribution cap"
                } else if raw == cap {
                    "Recorded operand is at contribution cap"
                } else {
                    "Recorded operand is below contribution cap"
                },
            ));
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intel_explanations_use_mod_values_native_decay_and_source_caps() {
        let defines = Defines::new().unwrap();
        defines.load("synthetic.lua",br#"NDefines={NIntel={COUNTRY_LEVEL_INTEL_MAXIMUMS={200,40,60,80},DYNAMIC_INTEL_SOURCE_EVENT_MAXIMUMS={},DYNAMIC_INTEL_SOURCE_EVENT_ABSOLUTE_MAXIMUMS={9,8,7,6},DYNAMIC_INTEL_SOURCE_EVENT_FLAT_DECAY=1,DYNAMIC_INTEL_SOURCE_EVENT_MULT_DECAY=0.5,ARMY_ARMY_COUNT_RANGE_INTEL_MIN=0.25,ARMY_ARMY_COUNT_RANGE_INTEL_MAX=0.75}}"#).unwrap();
        let row = IntelContributionRow {
            kind: 6,
            pool: 1,
            civilian: 500_000,
            army: -1,
            ..Default::default()
        };
        let explained = source(&defines, &row).unwrap();
        assert!(explained.iter().any(|v| v.value == "[200000, 0, 0, 0]"));
        assert!(
            source(&defines, &IntelContributionRow { pool: 7, ..row })
                .unwrap_err()
                .contains("unknown reached dynamic intel pool 7")
        );
        defines.load("mod-override.lua",br#"NDefines.NIntel.DYNAMIC_INTEL_SOURCE_EVENT_MULT_DECAY=2;NDefines.NIntel.DYNAMIC_INTEL_SOURCE_EVENT_ABSOLUTE_MAXIMUMS={}"#).unwrap();
        let grown = source(&defines, &row).unwrap();
        assert!(grown.iter().any(|v| v.value == "[800000, 0, 0, 0]"));
        assert!(
            grown
                .iter()
                .any(|v| v.value
                    == "[92233720200000, 92233720200000, 92233720200000, 92233720200000]")
        );
    }
}
