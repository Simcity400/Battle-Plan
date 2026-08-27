
use crate::*;
use std::collections::BTreeMap;

pub const STRATEGIC_REGIONS_DIR: &str = "map/strategicregions";
pub const SNOW_PROVINCES_BMP_SOURCE: &str = "map/provinces.bmp";
pub const SNOW_DEFINITION_CSV_SOURCE: &str = "map/definition.csv";

pub const SNOW_MUD_TEXTURE_WIDTH: usize = 1408;
pub const SNOW_MUD_TEXTURE_HEIGHT: usize = 512;
pub const SNOW_MUD_SOURCE_DXGI_FORMAT: u32 = 87;
pub const SNOW_MUD_TEXTURE_BYTE_LEN: usize = SNOW_MUD_TEXTURE_WIDTH * SNOW_MUD_TEXTURE_HEIGHT * 4;

pub const VANILLA_MAP_SIZE_X: f32 = 5632.0;
pub const VANILLA_MAP_SIZE_Y: f32 = 2048.0;

pub const CB0_OFFSET_SNOW_MUD_FADE_VECTOR: usize = 240;
pub const CB0_OFFSET_SNOW_MUD_FADE: usize = CB0_OFFSET_SNOW_MUD_FADE_VECTOR + 8;

pub const OBSERVED_MIN_SNOW_LEVELS: &[f32] = &[0.0, 0.01, 0.05, 0.1, 0.2, 0.3];


#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SnowDate {
    pub month: u8,
    pub day: u8,
}

impl SnowDate {
    pub fn new(month: u8, day: u8) -> Result<Self> {
        if (1..=12).contains(&month) && (1..=31).contains(&day) {
            Ok(Self { month, day })
        } else {
            Err(format!("invalid snow-law date {month:02}.{day:02}").into())
        }
    }

    fn nominal_ordinal(self) -> u32 {
        (u32::from(self.month) - 1) * 31 + (u32::from(self.day) - 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherPeriod {
    pub between_start: WeatherPoint,
    pub between_end: WeatherPoint,
    pub min_snow_level: f32,
    pub snow_probability: f32,
    pub blizzard_probability: f32,
    pub mud_probability: f32,
    pub temperature: [f32; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct WeatherPoint {
    pub day: u8,
    pub month0: u8,
}

impl WeatherPoint {
    fn parse(token: &str) -> Result<Self> {
        let (day_s, month_s) = token
            .split_once('.')
            .ok_or_else(|| format!("weather `between` point {token:?} is not DAY.MONTH"))?;
        let day = day_s
            .parse::<u8>()
            .map_err(|e| format!("weather `between` day {day_s:?}: {e}"))?;
        let month0 = month_s
            .parse::<u8>()
            .map_err(|e| format!("weather `between` month {month_s:?}: {e}"))?;
        if month0 > 11 {
            return Err(format!("weather `between` month {month0} out of range 0..=11").into());
        }
        if day > 30 {
            return Err(format!("weather `between` day {day} out of range 0..=30").into());
        }
        Ok(Self { day, month0 })
    }

    fn nominal_ordinal(self) -> u32 {
        u32::from(self.month0) * 31 + u32::from(self.day)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegionWeather {
    pub region_id: u32,
    pub name: String,
    pub provinces: Vec<u32>,
    pub periods: Vec<WeatherPeriod>,
}

impl RegionWeather {
    pub fn period_for_date(&self, date: SnowDate) -> Option<&WeatherPeriod> {
        let ord = date.nominal_ordinal();
        self.periods.iter().find(|p| period_contains(p, ord))
    }

    pub fn min_snow_floor_for_date(&self, date: SnowDate) -> Option<f32> {
        self.period_for_date(date).map(|p| p.min_snow_level)
    }
}

fn period_contains(period: &WeatherPeriod, ordinal: u32) -> bool {
    let start = period.between_start.nominal_ordinal();
    let end = period.between_end.nominal_ordinal();
    if start <= end {
        start <= ordinal && ordinal <= end
    } else {
        ordinal >= start || ordinal <= end
    }
}


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnowMudChannels {
    pub mud_fade0: u8,
    pub mud_fade1: u8,
    pub snow_fade0: u8,
    pub snow_fade1: u8,
}

impl SnowMudChannels {
    pub fn displayed_snow(self, fade: f32) -> f32 {
        let fade = fade.clamp(0.0, 1.0);
        let a = f32::from(self.snow_fade0) / 255.0;
        let b = f32::from(self.snow_fade1) / 255.0;
        a + (b - a) * fade
    }

    pub fn displayed_mud(self, fade: f32) -> f32 {
        let fade = fade.clamp(0.0, 1.0);
        let a = f32::from(self.mud_fade0) / 255.0;
        let b = f32::from(self.mud_fade1) / 255.0;
        a + (b - a) * fade
    }
}

pub fn decode_snow_mud_texel(payload: &[u8], texel_index: usize) -> Option<SnowMudChannels> {
    let base = texel_index.checked_mul(4)?;
    let end = base.checked_add(4)?;
    let px = payload.get(base..end)?;
    Some(SnowMudChannels {
        snow_fade0: px[0],
        snow_fade1: px[1],
        mud_fade0: px[2],
        mud_fade1: px[3],
    })
}

pub fn encode_snow_mud_texel(ch: SnowMudChannels) -> [u8; 4] {
    [ch.snow_fade0, ch.snow_fade1, ch.mud_fade0, ch.mud_fade1]
}

pub fn snow_mud_fade_from_cb0(cb0: &[u8]) -> Option<f32> {
    let end = CB0_OFFSET_SNOW_MUD_FADE.checked_add(4)?;
    let s = cb0.get(CB0_OFFSET_SNOW_MUD_FADE..end)?;
    Some(f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnowMudAddressing {
    pub map_size_x: f32,
    pub map_size_y: f32,
    pub fow_pow2_x: f32,
    pub fow_pow2_y: f32,
    pub texture_width: usize,
    pub texture_height: usize,
}

impl Default for SnowMudAddressing {
    fn default() -> Self {
        Self {
            map_size_x: VANILLA_MAP_SIZE_X,
            map_size_y: VANILLA_MAP_SIZE_Y,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            texture_width: SNOW_MUD_TEXTURE_WIDTH,
            texture_height: SNOW_MUD_TEXTURE_HEIGHT,
        }
    }
}

impl SnowMudAddressing {
    pub fn world_to_uv(self, x: f32, z: f32) -> (f32, f32) {
        let u = ((x + 0.5) / self.map_size_x) * self.fow_pow2_x;
        let v = ((z + 0.5) / self.map_size_y) * self.fow_pow2_y;
        (u, v)
    }

    pub fn world_to_texel(self, x: f32, z: f32) -> Option<(usize, usize)> {
        if self.texture_width == 0 || self.texture_height == 0 {
            return None;
        }
        let (u, v) = self.world_to_uv(x, z);
        let w = self.texture_width as f32;
        let h = self.texture_height as f32;
        let tx = wrap_index(u * w, self.texture_width);
        let ty = wrap_index(v * h, self.texture_height);
        Some((tx, ty))
    }
}

pub fn world_to_snow_mud_texel(x: f32, z: f32) -> Option<(usize, usize)> {
    SnowMudAddressing::default().world_to_texel(x, z)
}

fn wrap_index(coord: f32, dim: usize) -> usize {
    if dim == 0 {
        return 0;
    }
    let d = dim as i64;
    let mut i = coord.floor() as i64 % d;
    if i < 0 {
        i += d;
    }
    i as usize
}


pub fn region_weather_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<u32, RegionWeather>> {
    let files = resolver.resolve_dir_files(STRATEGIC_REGIONS_DIR, Some("txt"))?;
    let mut out = BTreeMap::new();
    for (_rel, path) in &files {
        let text = read_latin1(path)?;
        if let Some(region) = parse_region_weather(&text).map_err(|e| {
            format!(
                "failed to parse strategic region weather in {}: {e}",
                path.display()
            )
        })? {
            out.entry(region.region_id).or_insert(region);
        }
    }
    if out.is_empty() {
        return Err(
            format!("no strategic-region weather parsed under {STRATEGIC_REGIONS_DIR}").into(),
        );
    }
    Ok(out)
}

pub fn parse_region_weather(source: &str) -> Result<Option<RegionWeather>> {
    let text = strip_comments(source);
    let Some(region_id) = parse_u32_value(&text, "id") else {
        return Ok(None);
    };
    let name =
        parse_quoted_value(&text, "name").unwrap_or_else(|| format!("STRATEGICREGION_{region_id}"));
    let provinces = extract_block_after(&text, "provinces", 0)
        .map(|(inner, _)| parse_u32s(&inner))
        .unwrap_or_default();

    let periods = match extract_block_after(&text, "weather", 0) {
        Some((weather_inner, _)) => parse_weather_periods(&weather_inner)?,
        None => Vec::new(),
    };

    Ok(Some(RegionWeather {
        region_id,
        name,
        provinces,
        periods,
    }))
}

fn parse_weather_periods(weather_inner: &str) -> Result<Vec<WeatherPeriod>> {
    let mut periods = Vec::new();
    let mut cursor = 0usize;
    while let Some((period_inner, next)) = extract_block_after(weather_inner, "period", cursor) {
        periods.push(parse_weather_period(&period_inner)?);
        cursor = next;
    }
    Ok(periods)
}

fn parse_weather_period(period_inner: &str) -> Result<WeatherPeriod> {
    let (start_tok, end_tok) = parse_between(period_inner)?;
    let between_start = WeatherPoint::parse(&start_tok)?;
    let between_end = WeatherPoint::parse(&end_tok)?;
    let min_snow_level = parse_f32_value(period_inner, "min_snow_level").unwrap_or(0.0);
    let snow_probability = parse_f32_value(period_inner, "snow").unwrap_or(0.0);
    let blizzard_probability = parse_f32_value(period_inner, "blizzard").unwrap_or(0.0);
    let mud_probability = parse_f32_value(period_inner, "mud").unwrap_or(0.0);
    let temperature = parse_temperature(period_inner).unwrap_or([0.0, 0.0]);
    if !(0.0..=1.0).contains(&min_snow_level) {
        return Err(format!("min_snow_level {min_snow_level} out of [0,1]").into());
    }
    Ok(WeatherPeriod {
        between_start,
        between_end,
        min_snow_level,
        snow_probability,
        blizzard_probability,
        mud_probability,
        temperature,
    })
}

fn parse_between(period_inner: &str) -> Result<(String, String)> {
    let (inner, _) = extract_block_after(period_inner, "between", 0)
        .ok_or("weather period missing `between` block")?;
    let toks: Vec<&str> = inner.split_whitespace().collect();
    if toks.len() != 2 {
        return Err(format!("weather `between` expects two DAY.MONTH tokens, got {toks:?}").into());
    }
    Ok((toks[0].to_string(), toks[1].to_string()))
}

fn parse_temperature(period_inner: &str) -> Option<[f32; 2]> {
    let (inner, _) = extract_block_after(period_inner, "temperature", 0)?;
    let nums: Vec<f32> = inner
        .split_whitespace()
        .filter_map(|t| t.parse::<f32>().ok())
        .collect();
    if nums.len() == 2 {
        Some([nums[0], nums[1]])
    } else {
        None
    }
}


#[derive(Clone, Debug, PartialEq)]
pub struct SnowFloorTexture {
    pub width: usize,
    pub height: usize,
    pub floor: Vec<f32>,
    pub covered_texels: usize,
    pub regions_without_period: usize,
}

impl SnowFloorTexture {
    pub fn at(&self, tx: usize, ty: usize) -> f32 {
        if tx >= self.width || ty >= self.height {
            return 0.0;
        }
        self.floor[ty * self.width + tx]
    }

    pub fn to_snow_mud_payload(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.width * self.height * 4];
        for (i, &f) in self.floor.iter().enumerate() {
            let byte = (f.clamp(0.0, 1.0) * 255.0).round() as u8;
            let base = i * 4;
            out[base] = byte;
            out[base + 1] = byte;
        }
        out
    }
}

pub struct SnowFloorInputs<'a> {
    pub province_ids: &'a [u32],
    pub map_width: usize,
    pub map_height: usize,
    pub province_to_region: &'a BTreeMap<u32, u32>,
    pub region_weather: &'a BTreeMap<u32, RegionWeather>,
    pub addressing: SnowMudAddressing,
    pub flip_source_y: bool,
}

pub fn synthesize_snow_floor_from_resolver(
    date: SnowDate,
    resolver: &GameFileResolver,
) -> Result<SnowFloorTexture> {
    let region_weather = region_weather_from_resolver(resolver)?;
    let mut province_to_region: BTreeMap<u32, u32> = BTreeMap::new();
    for (id, rw) in &region_weather {
        for p in &rw.provinces {
            province_to_region.entry(*p).or_insert(*id);
        }
    }

    let definition_csv = resolver.resolve_required(SNOW_DEFINITION_CSV_SOURCE, "definition.csv")?;
    let provinces_bmp = resolver.resolve_required(SNOW_PROVINCES_BMP_SOURCE, "provinces.bmp")?;
    let (_definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;

    let inputs = SnowFloorInputs {
        province_ids: &bmp.ids,
        map_width: bmp.width,
        map_height: bmp.height,
        province_to_region: &province_to_region,
        region_weather: &region_weather,
        addressing: SnowMudAddressing::default(),
        flip_source_y: true,
    };
    synthesize_snow_floor_texture(date, &inputs)
}

pub fn synthesize_snow_floor_texture(
    date: SnowDate,
    inputs: &SnowFloorInputs<'_>,
) -> Result<SnowFloorTexture> {
    let width = inputs.addressing.texture_width;
    let height = inputs.addressing.texture_height;
    if width == 0 || height == 0 {
        return Err("snow floor synthesis: zero texture dimensions".into());
    }
    if inputs.province_ids.len() != inputs.map_width * inputs.map_height {
        return Err(format!(
            "snow floor synthesis: province raster {} != {}x{}",
            inputs.province_ids.len(),
            inputs.map_width,
            inputs.map_height
        )
        .into());
    }

    let mut region_floor: BTreeMap<u32, f32> = BTreeMap::new();
    let mut regions_without_period = 0usize;
    for (id, rw) in inputs.region_weather {
        if rw.periods.is_empty() {
            continue;
        }
        match rw.min_snow_floor_for_date(date) {
            Some(f) => {
                region_floor.insert(*id, f);
            }
            None => regions_without_period += 1,
        }
    }

    let mut floor = vec![0.0f32; width * height];
    let mut covered_texels = 0usize;
    let bx = (inputs.map_width as f32 / width as f32).max(1.0);
    let by = (inputs.map_height as f32 / height as f32).max(1.0);
    for ty in 0..height {
        for tx in 0..width {
            let mx = ((tx as f32 + 0.5) * bx) as usize;
            let unflipped_my = ((ty as f32 + 0.5) * by) as usize;
            if mx >= inputs.map_width || unflipped_my >= inputs.map_height {
                continue;
            }
            let my = if inputs.flip_source_y {
                inputs.map_height - 1 - unflipped_my.min(inputs.map_height - 1)
            } else {
                unflipped_my
            };
            let pid = inputs.province_ids[my * inputs.map_width + mx];
            let Some(region_id) = inputs.province_to_region.get(&pid) else {
                continue;
            };
            if let Some(&f) = region_floor.get(region_id) {
                if f > 0.0 {
                    floor[ty * width + tx] = f;
                    covered_texels += 1;
                }
            }
        }
    }

    Ok(SnowFloorTexture {
        width,
        height,
        floor,
        covered_texels,
        regions_without_period,
    })
}


#[derive(Clone, Debug, PartialEq)]
pub struct CapturedSnowMudStats {
    pub width: usize,
    pub height: usize,
    pub mud_fade0_nonzero: usize,
    pub mud_fade1_nonzero: usize,
    pub snow_fade0_nonzero: usize,
    pub snow_fade1_nonzero: usize,
    pub mud_fade0_max: u8,
    pub mud_fade1_max: u8,
    pub snow_fade0_max: u8,
    pub snow_fade1_max: u8,
    pub total_texels: usize,
}

pub fn captured_snow_mud_stats(
    payload: &[u8],
    width: usize,
    height: usize,
) -> Result<CapturedSnowMudStats> {
    let total = width
        .checked_mul(height)
        .ok_or("snow-mud stats: dimensions overflow")?;
    if payload.len() < total * 4 {
        return Err(format!(
            "snow-mud stats: payload {} shorter than {}x{}x4 = {}",
            payload.len(),
            width,
            height,
            total * 4
        )
        .into());
    }
    let mut s = CapturedSnowMudStats {
        width,
        height,
        mud_fade0_nonzero: 0,
        mud_fade1_nonzero: 0,
        snow_fade0_nonzero: 0,
        snow_fade1_nonzero: 0,
        mud_fade0_max: 0,
        mud_fade1_max: 0,
        snow_fade0_max: 0,
        snow_fade1_max: 0,
        total_texels: total,
    };
    for i in 0..total {
        let ch = decode_snow_mud_texel(payload, i).ok_or("snow-mud stats: short texel")?;
        if ch.mud_fade0 > 0 {
            s.mud_fade0_nonzero += 1;
        }
        if ch.mud_fade1 > 0 {
            s.mud_fade1_nonzero += 1;
        }
        if ch.snow_fade0 > 0 {
            s.snow_fade0_nonzero += 1;
        }
        if ch.snow_fade1 > 0 {
            s.snow_fade1_nonzero += 1;
        }
        s.mud_fade0_max = s.mud_fade0_max.max(ch.mud_fade0);
        s.mud_fade1_max = s.mud_fade1_max.max(ch.mud_fade1);
        s.snow_fade0_max = s.snow_fade0_max.max(ch.snow_fade0);
        s.snow_fade1_max = s.snow_fade1_max.max(ch.snow_fade1);
    }
    Ok(s)
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnowFloorResidual {
    pub total_texels: usize,
    pub floored_texels: usize,
    pub floor_respected: usize,
    pub floor_violations: usize,
    pub max_floor_shortfall: f32,
    pub exact_snow_byte_matches: usize,
    pub mean_abs_snow_diff: f32,
    pub fade: f32,
}

impl SnowFloorResidual {
    pub fn exact_match_percent(&self) -> f32 {
        if self.total_texels == 0 {
            0.0
        } else {
            100.0 * self.exact_snow_byte_matches as f32 / self.total_texels as f32
        }
    }
    pub fn floor_respected_percent(&self) -> f32 {
        if self.floored_texels == 0 {
            100.0
        } else {
            100.0 * self.floor_respected as f32 / self.floored_texels as f32
        }
    }
}

pub fn compare_floor_to_capture(
    floor: &SnowFloorTexture,
    payload: &[u8],
    fade: f32,
) -> Result<SnowFloorResidual> {
    let total = floor.width * floor.height;
    if payload.len() < total * 4 {
        return Err(format!(
            "compare: payload {} shorter than {}x{}x4",
            payload.len(),
            floor.width,
            floor.height
        )
        .into());
    }
    let mut r = SnowFloorResidual {
        total_texels: total,
        floored_texels: 0,
        floor_respected: 0,
        floor_violations: 0,
        max_floor_shortfall: 0.0,
        exact_snow_byte_matches: 0,
        mean_abs_snow_diff: 0.0,
        fade,
    };
    let tol = 1.0 / 255.0;
    let mut sum_abs = 0.0f64;
    for i in 0..total {
        let ch = decode_snow_mud_texel(payload, i).ok_or("compare: short texel")?;
        let captured_snow = ch.displayed_snow(fade);
        // The floor may be carried in either endpoint; the lower-bound check
        let endpoint_max = f32::from(ch.snow_fade0.max(ch.snow_fade1)) / 255.0;
        let floor_val = floor.floor[i];
        let floor_byte = (floor_val.clamp(0.0, 1.0) * 255.0).round() as u8;
        let captured_byte = (captured_snow.clamp(0.0, 1.0) * 255.0).round() as u8;
        if floor_byte == captured_byte {
            r.exact_snow_byte_matches += 1;
        }
        sum_abs += (captured_snow - floor_val).abs() as f64;
        if floor_val > 0.0 {
            r.floored_texels += 1;
            if endpoint_max + tol >= floor_val {
                r.floor_respected += 1;
            } else {
                r.floor_violations += 1;
                r.max_floor_shortfall = r.max_floor_shortfall.max(floor_val - endpoint_max);
            }
        }
    }
    r.mean_abs_snow_diff = if total == 0 {
        0.0
    } else {
        (sum_abs / total as f64) as f32
    };
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGION_SRC: &str = r#"
strategic_region={
	id=42
	name="STRATEGICREGION_42"
	provinces={
		100 101 102
	}
	weather={
		period={
			between={ 0.0 30.0 }
			temperature={ -40.0 -12.0 }
			snow=0.350
			blizzard=0.300
			mud=0.500
			min_snow_level=0.300
		}
		period={
			between={ 0.1 27.1 }
			temperature={ -7.0 12.0 }
			snow=0.200
			blizzard=0.050
			mud=0.300
			min_snow_level=0.100
		}
		period={
			between={ 0.10 29.10 }
			temperature={ -1.0 15.0 }
			snow=0.210
			blizzard=0.000
			mud=0.300
			min_snow_level=0.050
		}
		period={
			between={ 0.11 30.11 }
			temperature={ -5.0 13.0 }
			snow=0.200
			blizzard=0.000
			mud=0.300
			min_snow_level=0.200
		}
	}
}
"#;

    fn parsed_region() -> RegionWeather {
        parse_region_weather(REGION_SRC)
            .expect("parse ok")
            .expect("has id")
    }

    #[test]
    fn parses_id_name_provinces_and_periods() {
        let r = parsed_region();
        assert_eq!(r.region_id, 42);
        assert_eq!(r.name, "STRATEGICREGION_42");
        assert_eq!(r.provinces, vec![100, 101, 102]);
        assert_eq!(r.periods.len(), 4);
        assert_eq!(r.periods[0].min_snow_level, 0.3);
        assert_eq!(r.periods[0].temperature, [-40.0, -12.0]);
        assert_eq!(r.periods[0].snow_probability, 0.35);
    }

    #[test]
    fn between_day_month_zero_based_parse() {
        let p0 = WeatherPoint::parse("0.0").unwrap();
        assert_eq!(p0, WeatherPoint { day: 0, month0: 0 });
        let p1 = WeatherPoint::parse("29.10").unwrap();
        assert_eq!(
            p1,
            WeatherPoint {
                day: 29,
                month0: 10
            }
        );
        let p2 = WeatherPoint::parse("30.11").unwrap();
        assert_eq!(
            p2,
            WeatherPoint {
                day: 30,
                month0: 11
            }
        );
    }

    #[test]
    fn period_matching_selects_month_by_date() {
        let r = parsed_region();
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(1, 5).unwrap()),
            Some(0.3)
        );
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(2, 10).unwrap()),
            Some(0.1)
        );
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(11, 15).unwrap()),
            Some(0.05)
        );
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(12, 20).unwrap()),
            Some(0.2)
        );
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(3, 15).unwrap()),
            None
        );
    }

    #[test]
    fn period_matching_handles_year_wrap_window() {
        let src = r#"
strategic_region={
	id=7
	provinces={ 1 }
	weather={
		period={
			between={ 14.11 14.0 }
			min_snow_level=0.5
		}
	}
}
"#;
        let r = parse_region_weather(src).unwrap().unwrap();
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(12, 20).unwrap()),
            Some(0.5)
        );
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(1, 10).unwrap()),
            Some(0.5)
        );
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(6, 1).unwrap()),
            None
        );
    }

    #[test]
    fn no_weather_block_yields_empty_periods_not_error() {
        let src = "strategic_region={ id=9 provinces={ 5 } }";
        let r = parse_region_weather(src).unwrap().unwrap();
        assert!(r.periods.is_empty());
        assert_eq!(
            r.min_snow_floor_for_date(SnowDate::new(1, 1).unwrap()),
            None
        );
    }

    #[test]
    fn malformed_between_fails_closed() {
        let src = r#"
strategic_region={ id=3 provinces={ 1 }
	weather={ period={ between={ 0.0 } min_snow_level=0.1 } } }
"#;
        let err = parse_region_weather(src).expect_err("one-token between fails");
        assert!(err.to_string().contains("between"), "{err}");
    }

    #[test]
    fn min_snow_out_of_range_fails_closed() {
        let src = r#"
strategic_region={ id=3 provinces={ 1 }
	weather={ period={ between={ 0.0 30.0 } min_snow_level=1.5 } } }
"#;
        let err = parse_region_weather(src).expect_err("min_snow 1.5 fails");
        assert!(err.to_string().contains("out of [0,1]"), "{err}");
    }

    #[test]
    fn channel_decode_uses_bgra_byte_order() {
        let payload = [10u8, 20, 30, 40];
        let ch = decode_snow_mud_texel(&payload, 0).unwrap();
        assert_eq!(ch.snow_fade0, 10);
        assert_eq!(ch.snow_fade1, 20);
        assert_eq!(ch.mud_fade0, 30);
        assert_eq!(ch.mud_fade1, 40);
        assert_eq!(encode_snow_mud_texel(ch), payload);
    }

    #[test]
    fn displayed_snow_lerps_fade0_to_fade1_by_fade() {
        let ch = SnowMudChannels {
            snow_fade0: 0,
            snow_fade1: 255,
            mud_fade0: 0,
            mud_fade1: 0,
        };
        assert_eq!(ch.displayed_snow(0.0), 0.0);
        assert_eq!(ch.displayed_snow(1.0), 1.0);
        assert!((ch.displayed_snow(0.5) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn decode_and_fade_fail_closed_on_short_buffers() {
        assert!(decode_snow_mud_texel(&[1, 2, 3], 0).is_none());
        assert!(snow_mud_fade_from_cb0(&[0u8; CB0_OFFSET_SNOW_MUD_FADE]).is_none());
        assert!(snow_mud_fade_from_cb0(&[0u8; CB0_OFFSET_SNOW_MUD_FADE + 4]).is_some());
    }

    #[test]
    fn cb0_snow_mud_fade_reads_correct_offset() {
        let mut cb0 = vec![0u8; 416];
        cb0[CB0_OFFSET_SNOW_MUD_FADE..CB0_OFFSET_SNOW_MUD_FADE + 4]
            .copy_from_slice(&0.75f32.to_le_bytes());
        assert_eq!(snow_mud_fade_from_cb0(&cb0), Some(0.75));
    }

    #[test]
    fn addressing_maps_world_to_quarter_res_texel() {
        let a = SnowMudAddressing::default();
        assert_eq!(a.world_to_texel(0.0, 0.0), Some((0, 0)));
        let (tx, ty) = a
            .world_to_texel(VANILLA_MAP_SIZE_X / 2.0, VANILLA_MAP_SIZE_Y / 2.0)
            .unwrap();
        assert!(
            (tx as i32 - (SNOW_MUD_TEXTURE_WIDTH as i32 / 2)).abs() <= 1,
            "tx={tx}"
        );
        assert!(
            (ty as i32 - (SNOW_MUD_TEXTURE_HEIGHT as i32 / 2)).abs() <= 1,
            "ty={ty}"
        );
        let (tx, ty) = a
            .world_to_texel(VANILLA_MAP_SIZE_X - 1.0, VANILLA_MAP_SIZE_Y - 1.0)
            .unwrap();
        assert_eq!(
            (tx, ty),
            (SNOW_MUD_TEXTURE_WIDTH - 1, SNOW_MUD_TEXTURE_HEIGHT - 1)
        );
    }

    #[test]
    fn addressing_wraps_out_of_range_like_wrap_sampler() {
        let a = SnowMudAddressing::default();
        assert_eq!(
            a.world_to_texel(VANILLA_MAP_SIZE_X, 0.0).map(|t| t.0),
            Some(0)
        );
        let neg = a.world_to_texel(-1.0, 0.0).unwrap().0;
        assert_eq!(neg, SNOW_MUD_TEXTURE_WIDTH - 1);
    }

    fn tiny_floor_inputs() -> (BTreeMap<u32, RegionWeather>, BTreeMap<u32, u32>, Vec<u32>) {
        let mut rw = BTreeMap::new();
        rw.insert(42, parsed_region());
        let mut p2r = BTreeMap::new();
        for pid in [100u32, 101, 102] {
            p2r.insert(pid, 42);
        }
        let mut ids = vec![999u32; 64];
        for y in 0..8 {
            for x in 0..4 {
                ids[y * 8 + x] = 100;
            }
        }
        (rw, p2r, ids)
    }

    #[test]
    fn floor_synthesis_covers_region_texels_only() {
        let (rw, p2r, ids) = tiny_floor_inputs();
        let addressing = SnowMudAddressing {
            map_size_x: 8.0,
            map_size_y: 8.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            texture_width: 4,
            texture_height: 4,
        };
        let inputs = SnowFloorInputs {
            province_ids: &ids,
            map_width: 8,
            map_height: 8,
            province_to_region: &p2r,
            region_weather: &rw,
            addressing,
            flip_source_y: false,
        };
        let tex = synthesize_snow_floor_texture(SnowDate::new(1, 5).unwrap(), &inputs).unwrap();
        assert_eq!(tex.width, 4);
        assert_eq!(tex.height, 4);
        assert_eq!(tex.at(0, 0), 0.3);
        assert_eq!(tex.at(1, 0), 0.3);
        assert_eq!(tex.at(2, 0), 0.0);
        assert_eq!(tex.at(3, 0), 0.0);
        assert_eq!(tex.covered_texels, 4 * 2);
    }

    #[test]
    fn floor_synthesis_fails_closed_when_no_period_matches_date() {
        let (rw, p2r, ids) = tiny_floor_inputs();
        let addressing = SnowMudAddressing {
            map_size_x: 8.0,
            map_size_y: 8.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            texture_width: 4,
            texture_height: 4,
        };
        let inputs = SnowFloorInputs {
            province_ids: &ids,
            map_width: 8,
            map_height: 8,
            province_to_region: &p2r,
            region_weather: &rw,
            addressing,
            flip_source_y: false,
        };
        let tex = synthesize_snow_floor_texture(SnowDate::new(3, 15).unwrap(), &inputs).unwrap();
        assert_eq!(tex.covered_texels, 0);
        assert_eq!(tex.regions_without_period, 1);
        assert!(tex.floor.iter().all(|&f| f == 0.0));
    }

    #[test]
    fn floor_synthesis_flip_source_y_toggle_controls_orientation() {
        let mut rw = BTreeMap::new();
        rw.insert(42, parsed_region());
        let mut p2r = BTreeMap::new();
        p2r.insert(100u32, 42);
        let mut ids = vec![999u32; 64];
        for y in 0..4 {
            for x in 0..8 {
                ids[y * 8 + x] = 100;
            }
        }
        let addressing = SnowMudAddressing {
            map_size_x: 8.0,
            map_size_y: 8.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            texture_width: 4,
            texture_height: 4,
        };
        let flipped = SnowFloorInputs {
            province_ids: &ids,
            map_width: 8,
            map_height: 8,
            province_to_region: &p2r,
            region_weather: &rw,
            addressing,
            flip_source_y: true,
        };
        let tex = synthesize_snow_floor_texture(SnowDate::new(1, 5).unwrap(), &flipped).unwrap();
        assert_eq!(
            tex.at(0, 0),
            0.0,
            "flipped: top texture row is uncovered south"
        );
        assert_eq!(
            tex.at(0, 3),
            0.3,
            "flipped: bottom texture row carries the north floor"
        );

        let unflipped = SnowFloorInputs {
            province_ids: &ids,
            map_width: 8,
            map_height: 8,
            province_to_region: &p2r,
            region_weather: &rw,
            addressing,
            flip_source_y: false,
        };
        let tex2 = synthesize_snow_floor_texture(SnowDate::new(1, 5).unwrap(), &unflipped).unwrap();
        assert_eq!(
            tex2.at(0, 0),
            0.3,
            "unflipped: top texture row carries the north floor"
        );
        assert_eq!(
            tex2.at(0, 3),
            0.0,
            "unflipped: bottom texture row is uncovered south"
        );
    }

    #[test]
    fn floor_payload_writes_snow_into_b_and_g_only() {
        let tex = SnowFloorTexture {
            width: 2,
            height: 1,
            floor: vec![0.2, 0.0],
            covered_texels: 1,
            regions_without_period: 0,
        };
        let payload = tex.to_snow_mud_payload();
        let byte = (0.2f32 * 255.0).round() as u8;
        assert_eq!(payload[0], byte);
        assert_eq!(payload[1], byte);
        assert_eq!(payload[2], 0);
        assert_eq!(payload[3], 0);
        assert_eq!(&payload[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn floor_respected_against_a_capture_that_meets_the_floor() {
        let floor = SnowFloorTexture {
            width: 1,
            height: 1,
            floor: vec![0.3],
            covered_texels: 1,
            regions_without_period: 0,
        };
        let payload = [128u8, 0, 0, 0];
        let r = compare_floor_to_capture(&floor, &payload, 0.0).unwrap();
        assert_eq!(r.floored_texels, 1);
        assert_eq!(r.floor_respected, 1);
        assert_eq!(r.floor_violations, 0);
        assert_eq!(r.floor_respected_percent(), 100.0);
    }

    #[test]
    fn floor_violation_detected_when_capture_below_floor() {
        let floor = SnowFloorTexture {
            width: 1,
            height: 1,
            floor: vec![0.3],
            covered_texels: 1,
            regions_without_period: 0,
        };
        let payload = [0u8, 0, 0, 0];
        let r = compare_floor_to_capture(&floor, &payload, 0.0).unwrap();
        assert_eq!(r.floor_violations, 1);
        assert!((r.max_floor_shortfall - 0.3).abs() < 1e-3);
    }

    #[test]
    fn captured_stats_decode_channel_maxes() {
        let payload = [10u8, 20, 30, 40, 0, 0, 0, 0];
        let s = captured_snow_mud_stats(&payload, 2, 1).unwrap();
        assert_eq!(s.total_texels, 2);
        assert_eq!(s.snow_fade0_nonzero, 1);
        assert_eq!(s.snow_fade1_max, 20);
        assert_eq!(s.mud_fade1_max, 40);
    }
}
