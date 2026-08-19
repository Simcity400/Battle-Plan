
use crate::mod_resolver::GameFileResolver;
use crate::Result;

pub const OCCUPATION_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OccupationColorStop {
    pub level: f32,
    pub rgba: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationMapModePalette {
    pub non_initialized: [f32; 4],
    pub resistance: Vec<OccupationColorStop>,
    pub compliance: Vec<OccupationColorStop>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccupationMapMode {
    Resistance,
    Compliance,
}

pub fn occupation_palette_from_source(source: &str) -> Result<OccupationMapModePalette> {
    Ok(OccupationMapModePalette {
        non_initialized: parse_color4(source, "OCCUPATION_RESISTANCE_NON_INITIALIZED_COLOR")?,
        resistance: parse_stop_table(source, "OCCUPATION_RESISTANCE_MAP_MODE_COLORS")?,
        compliance: parse_stop_table(source, "OCCUPATION_COMPLIANCE_MAP_MODE_COLORS")?,
    })
}

pub fn occupation_palette_from_resolver(
    resolver: &GameFileResolver,
) -> Result<OccupationMapModePalette> {
    let path = resolver.resolve_required(
        OCCUPATION_GRAPHICS_DEFINES_SOURCE,
        "occupation resistance/compliance map-mode colors",
    )?;
    occupation_palette_from_source(&crate::read_latin1(&path)?)
}

pub fn occupation_mapmode_color(
    palette: &OccupationMapModePalette,
    mode: OccupationMapMode,
    level: f32,
) -> Option<[f32; 4]> {
    let stops = match mode {
        OccupationMapMode::Resistance => &palette.resistance,
        OccupationMapMode::Compliance => &palette.compliance,
    };
    interpolate(stops, level)
}

fn interpolate(stops: &[OccupationColorStop], level: f32) -> Option<[f32; 4]> {
    if stops.is_empty() || !level.is_finite() {
        return None;
    }
    let lower = stops.iter().rposition(|stop| stop.level <= level);
    let Some(lower) = lower else {
        return Some(stops[0].rgba);
    };
    if lower + 1 == stops.len() {
        return Some(stops[lower].rgba);
    }
    let a = stops[lower];
    let b = stops[lower + 1];
    let span = b.level - a.level;
    if span <= 0.0 {
        return Some(b.rgba);
    }
    let t = ((level - a.level) / span).clamp(0.0, 1.0);
    Some(std::array::from_fn(|i| {
        a.rgba[i] + (b.rgba[i] - a.rgba[i]) * t
    }))
}

fn parse_color4(source: &str, key: &str) -> Result<[f32; 4]> {
    let values = parse_table_numbers(source, key)?;
    if values.len() != 4 {
        return Err(format!(
            "{OCCUPATION_GRAPHICS_DEFINES_SOURCE}: {key} expected 4 values, got {}",
            values.len()
        )
        .into());
    }
    Ok([values[0], values[1], values[2], values[3]])
}

fn parse_stop_table(source: &str, key: &str) -> Result<Vec<OccupationColorStop>> {
    let values = parse_table_numbers(source, key)?;
    if values.len() < 10 || values.len() % 5 != 0 {
        return Err(format!(
            "{OCCUPATION_GRAPHICS_DEFINES_SOURCE}: {key} expected complete five-value stops, got {} values",
            values.len()
        )
        .into());
    }
    let stops = values
        .chunks_exact(5)
        .map(|row| OccupationColorStop {
            level: row[0],
            rgba: [row[1], row[2], row[3], row[4]],
        })
        .collect::<Vec<_>>();
    if stops.windows(2).any(|pair| pair[0].level > pair[1].level)
        || stops.iter().any(|stop| {
            !stop.level.is_finite()
                || stop
                    .rgba
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        })
    {
        return Err(format!(
            "{OCCUPATION_GRAPHICS_DEFINES_SOURCE}: {key} contains invalid or descending stops"
        )
        .into());
    }
    Ok(stops)
}

fn parse_table_numbers(source: &str, key: &str) -> Result<Vec<f32>> {
    let source = source
        .lines()
        .map(|line| line.split("--").next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    let key_pos = source
        .find(key)
        .ok_or_else(|| format!("{OCCUPATION_GRAPHICS_DEFINES_SOURCE}: missing {key}"))?;
    let after = &source[key_pos + key.len()..];
    let open_rel = after.find('{').ok_or_else(|| {
        format!("{OCCUPATION_GRAPHICS_DEFINES_SOURCE}: {key} missing opening brace")
    })?;
    let body = &after[open_rel + 1..];
    let close_rel = body.find('}').ok_or_else(|| {
        format!("{OCCUPATION_GRAPHICS_DEFINES_SOURCE}: {key} missing closing brace")
    })?;
    body[..close_rel]
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| {
            token.parse::<f32>().map_err(|_| {
                format!("{OCCUPATION_GRAPHICS_DEFINES_SOURCE}: {key} invalid number {token:?}")
                    .into()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"
      OCCUPATION_RESISTANCE_NON_INITIALIZED_COLOR = { 1.0, 1.0, 1.0, 0.05 },
      OCCUPATION_RESISTANCE_MAP_MODE_COLORS = {
        0.0, 0.0, 1.0, 0.0, 0.0,
        0.0, 1.0, 1.0, 0.0, 0.0,
        1.0, 1.0, 1.0, 0.0, 0.1,
        30.0, 1.0, 1.0, 0.0, 0.3,
        100.0, 1.0, 0.0, 0.0, 0.3,
      },
      OCCUPATION_COMPLIANCE_MAP_MODE_COLORS = {
        0.0, 0.3, 0.6, 0.6, 0.05,
        0.0, 0.3, 0.7, 1.0, 0.05,
        10.0, 0.3, 0.7, 1.0, 0.2,
        50.0, 0.3, 0.7, 1.0, 0.3,
        100.0, 0.3, 0.9, 1.0, 0.5,
      },
    "#;

    #[test]
    fn parses_duplicate_zero_stops_and_interpolates_exactly() {
        let palette = occupation_palette_from_source(SOURCE).unwrap();
        assert_eq!(palette.resistance.len(), 5);
        assert_eq!(
            occupation_mapmode_color(&palette, OccupationMapMode::Resistance, 0.0),
            Some([1.0, 1.0, 0.0, 0.0])
        );
        let mid = occupation_mapmode_color(&palette, OccupationMapMode::Resistance, 15.5)
            .expect("midpoint color");
        assert_eq!(&mid[..3], &[1.0, 1.0, 0.0]);
        assert!((mid[3] - 0.2).abs() < 1.0e-6);
        assert_eq!(
            occupation_mapmode_color(&palette, OccupationMapMode::Compliance, 100.0),
            Some([0.3, 0.9, 1.0, 0.5])
        );
    }
}
