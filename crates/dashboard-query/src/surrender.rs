use content_reader_defines::defines::Defines;
use replay_data::rows::{CapitulationRow, CountryGameScoreInputRow, Recorded};

const ONE: i64 = 100_000;

#[derive(Debug, PartialEq, Eq)]
pub struct DerivedProgress(pub i64);

fn required<T: Copy>(input: &Recorded<Option<T>>, name: &str) -> Result<T, String> {
    match input {
        Recorded::Value(Some(value)) => Ok(*value),
        _ => Err(format!("Capitulation.{name} not recorded")),
    }
}

pub fn progress(
    row: &CapitulationRow,
    country: &CountryGameScoreInputRow,
    defines: &Defines,
) -> Result<DerivedProgress, String> {
    let core = required(&row.cached_total_core_weight, "cached_total_core_weight")?;
    let held_core = required(&row.cached_held_core_weight, "cached_held_core_weight")?;
    let total = required(
        &row.cached_total_territory_weight,
        "cached_total_territory_weight",
    )?;
    let held = required(
        &row.cached_held_territory_weight,
        "cached_held_territory_weight",
    )?;
    let base_adjustment = required(&row.surrender_limit_modifier, "surrender_limit_modifier")?;
    let final_adjustment = required(
        &row.forced_surrender_limit_modifier,
        "forced_surrender_limit_modifier",
    )?;
    let upper_offset = required(
        &row.max_surrender_limit_offset,
        "max_surrender_limit_offset",
    )?;
    let collaboration = required(&row.selected_collaboration, "selected_collaboration")?;
    let controlled = country.land_count.ok_or_else(|| {
        format!(
            "CountryGameScoreInputs.land_count not recorded ({})",
            String::from_utf8_lossy(&country.land_error)
        )
    })?;
    let define = |name| {
        defines
            .fixed5("NCountry", name)
            .map_err(|error| error.to_string())
    };
    let base = define("BASE_SURRENDER_LIMIT")?;
    let minimum = define("MIN_SURRENDER_LIMIT")?;
    let reduction = define("SURRENDER_LIMIT_REDUCTION_PER_COLLABORATION")?;
    let no_cores = define("SURRENDER_LIMIT_MULT_FOR_COUNTRIES_WITH_NO_CORES")?;
    if controlled == 0 {
        return Ok(DerivedProgress(ONE));
    }
    let upper = minimum.max(ONE.wrapping_sub(upper_offset));
    let mut limit = base
        .wrapping_add(base_adjustment)
        .wrapping_sub(collaboration.wrapping_mul(reduction) / ONE);
    if core == 0 {
        limit = limit.wrapping_mul(no_cores) / ONE;
    }
    limit = limit
        .clamp(minimum, upper)
        .wrapping_add(final_adjustment)
        .min(ONE);
    if limit <= 0 {
        return Ok(DerivedProgress(ONE));
    }
    let (held, total) = if core > 0 {
        (held_core, core)
    } else {
        (held, total)
    };
    let occupied = if total > 0 {
        let share = i64::from(held).wrapping_mul(ONE * ONE) / (i64::from(total) * ONE);
        (ONE - share).clamp(0, ONE)
    } else {
        0
    };
    Ok(DerivedProgress((occupied * ONE / limit).min(ONE)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surrender_progress_uses_hourly_cached_weights_and_archived_limits() {
        let defines = Defines::new().unwrap();
        defines
            .load(
                "common/defines/fixture.lua",
                br#"NDefines = { NCountry = {
            BASE_SURRENDER_LIMIT = 0.675, MIN_SURRENDER_LIMIT = 0.175,
            SURRENDER_LIMIT_REDUCTION_PER_COLLABORATION = 0.33,
            SURRENDER_LIMIT_MULT_FOR_COUNTRIES_WITH_NO_CORES = 0.625 } }"#,
            )
            .unwrap();
        let mut inputs = CapitulationRow {
            cached_held_core_weight: Some(81).into(),
            cached_total_core_weight: Some(100).into(),
            cached_held_territory_weight: Some(14).into(),
            cached_total_territory_weight: Some(20).into(),
            surrender_limit_modifier: Some(0).into(),
            forced_surrender_limit_modifier: Some(2_500).into(),
            max_surrender_limit_offset: Some(45_000).into(),
            selected_collaboration: Some(50_000).into(),
            ..Default::default()
        };
        let mut territory = CountryGameScoreInputRow {
            land_count: Some(3),
            ..Default::default()
        };
        assert_eq!(
            progress(&inputs, &territory, &defines),
            Ok(DerivedProgress(35_514))
        );
        inputs.cached_total_core_weight = Some(0).into();
        assert_eq!(
            progress(&inputs, &territory, &defines),
            Ok(DerivedProgress(87_272))
        );
        inputs.forced_surrender_limit_modifier = Some(-80_000).into();
        assert_eq!(
            progress(&inputs, &territory, &defines),
            Ok(DerivedProgress(ONE))
        );
        inputs.forced_surrender_limit_modifier = Some(0).into();
        territory.land_count = Some(0);
        assert_eq!(
            progress(&inputs, &territory, &defines),
            Ok(DerivedProgress(ONE))
        );
        for missing in [Recorded::NotRecorded, Recorded::Value(None)] {
            inputs.cached_held_territory_weight = missing;
            assert_eq!(
                progress(&inputs, &territory, &defines).unwrap_err(),
                "Capitulation.cached_held_territory_weight not recorded"
            );
        }
    }
}
