use super::localise;
use installed_content::{
    localisation::{Argument as Arg, Localisation},
    weather::TemperatureRange,
};
use replay::rows::{WeatherProvinceRow, WeatherRegionRow};

/// Independent weather flags, first matching thermal range, with the
/// temperature the province's record holds at the frame's hour.
pub(super) fn row(
    loc: &Localisation,
    ranges: &[TemperatureRange; 4],
    temperature: Result<i64, String>,
    province: &WeatherProvinceRow,
    region: &WeatherRegionRow,
) -> Result<String, String> {
    let temperature = temperature?;
    let mud = *province.raw_mud.require("WeatherProvinces.raw_mud")?;
    let flags = [
        region.light_rain,
        region.heavy_rain,
        region.snow,
        region.blizzard,
        region.sandstorm,
        mud,
    ];
    let mut fragments = String::new();
    for (i, (flag, key)) in flags
        .into_iter()
        .zip([
            "weather_rain_light",
            "weather_rain_heavy",
            "weather_snow",
            "weather_blizzard",
            "weather_sandstorm",
            "weather_mud",
        ])
        .enumerate()
    {
        if flag == 0 {
            continue;
        }
        let name = localise(loc, key, &[])?;
        fragments.push_str(&localise(
            loc,
            "PROV_TOOLTIP_WEATHER_MODIFIER",
            &[
                ("FRAME", Arg::Int32(i as i32 + 1)),
                ("TEXT", Arg::Text(&name)),
            ],
        )?);
        fragments.push(' ');
    }
    if let Some((i, range)) = ranges
        .iter()
        .enumerate()
        .find(|(_, r)| r.lower <= temperature && temperature < r.upper)
    {
        let name = range
            .modifier
            .as_ref()
            .map(|key| localise(loc, key, &[]))
            .transpose()?
            .unwrap_or_default();
        fragments.push_str(&localise(
            loc,
            "PROV_TOOLTIP_TEMPERATURE_MODIFIER",
            &[
                ("FRAME", Arg::Int32(i as i32 + 1)),
                ("TEXT", Arg::Text(&name)),
            ],
        )?);
        fragments.push(' ');
    }
    if fragments.is_empty() {
        fragments = format!("\u{11}Y{} \u{11}!", localise(loc, "weather_none", &[])?);
    }
    // Copy fragments into TEXT, then append the result.
    // A missing nested WEATHER key is empty, not a TEXT alias.
    let temperature = localise(
        loc,
        "PROV_TOOLTIP_TEMPERATURE",
        &[
            ("TEXT", Arg::Text(&fragments)),
            ("TEMP", Arg::Fixed5(temperature)),
        ],
    )?;
    Ok(format!(
        "{} {fragments}{temperature}",
        localise(loc, "PROVINCE_WEATHER_HEADER", &[])?
    ))
}

#[cfg(test)]
mod tests;
