//! Province geometry and authored country colors from verified game content.

use std::{
    collections::{BTreeMap, HashMap},
    io,
};

use crate::{
    ResolvedLayers,
    content::normalize_relative,
    parser::{self, Item, Value},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvinceKind {
    Land,
    Sea,
    Lake,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceDefinition {
    pub id: u32,
    pub rgb: [u8; 3],
    pub kind: ProvinceKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryColors {
    /// Base content colors, including colors.txt overrides. Runtime and cosmetic
    /// overrides are not inferred from campaign history or country identity.
    pub rgb: BTreeMap<Vec<u8>, [u8; 3]>,
    pub unavailable: BTreeMap<Vec<u8>, String>,
}

impl CountryColors {
    fn assign(&mut self, tag: Vec<u8>, color: io::Result<[u8; 3]>) {
        match color {
            Ok(color) => {
                self.unavailable.remove(&tag);
                self.rgb.insert(tag, color);
            }
            Err(error) => {
                self.rgb.remove(&tag);
                self.unavailable.insert(tag, error.to_string());
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceImage {
    pub width: u32,
    pub height: u32,
    /// Row-major pixels, starting at the top-left regardless of BMP storage order.
    pub province_ids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapData {
    pub width: u32,
    pub height: u32,
    pub province_ids: Vec<u32>,
    /// Includes the map's zero sentinel if definition.csv declares it.
    pub definitions: BTreeMap<u32, ProvinceDefinition>,
    pub country_colors: CountryColors,
}

fn integer<T: std::str::FromStr>(text: &str, field: &str) -> io::Result<T> {
    text.trim()
        .parse()
        .map_err(|_| invalid(format!("invalid {field}: {text:?}")))
}

pub fn parse_definitions(input: &[u8]) -> io::Result<BTreeMap<u32, ProvinceDefinition>> {
    let input = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input);
    let text = std::str::from_utf8(input).map_err(|_| invalid("definition.csv is not UTF-8"))?;
    let mut definitions = BTreeMap::new();
    let mut colors = BTreeMap::new();
    for (line_number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split(';').map(str::trim).collect::<Vec<_>>();
        if fields.len() < 5 {
            return Err(invalid(format!(
                "definition.csv line {} lacks id/RGB/kind columns",
                line_number + 1
            )));
        }
        let id = integer(fields[0], "province id")?;
        let rgb = [
            integer(fields[1], "province red")?,
            integer(fields[2], "province green")?,
            integer(fields[3], "province blue")?,
        ];
        let kind = match fields[4] {
            "land" => ProvinceKind::Land,
            "sea" => ProvinceKind::Sea,
            "lake" => ProvinceKind::Lake,
            other => {
                return Err(invalid(format!(
                    "unknown province kind {other:?} on line {}",
                    line_number + 1
                )));
            }
        };
        if definitions
            .insert(id, ProvinceDefinition { id, rgb, kind })
            .is_some()
        {
            return Err(invalid(format!("duplicate province id {id}")));
        }
        if let Some(previous) = colors.insert(rgb, id) {
            return Err(invalid(format!(
                "provinces {previous} and {id} use duplicate RGB {rgb:?}"
            )));
        }
    }
    if definitions.is_empty() {
        return Err(invalid("definition.csv contains no provinces"));
    }
    Ok(definitions)
}

fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("truncated BMP header"))?
            .try_into()
            .unwrap(),
    ))
}

fn u16_at(bytes: &[u8], offset: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| invalid("truncated BMP header"))?
            .try_into()
            .unwrap(),
    ))
}

/// Reads BI_RGB 24-bit Windows BMP pixels directly into province ids. Padding
/// and BGR channel order are storage details; they never become map pixels.
pub fn decode_provinces_bmp(
    input: &[u8],
    definitions: &BTreeMap<u32, ProvinceDefinition>,
) -> io::Result<ProvinceImage> {
    if input.get(..2) != Some(b"BM") {
        return Err(invalid("province image is not a BMP"));
    }
    if u32_at(input, 2)? as usize != input.len() {
        return Err(invalid("BMP declared size does not match its bytes"));
    }
    let offset = usize::try_from(u32_at(input, 10)?)
        .map_err(|_| invalid("BMP offset exceeds address space"))?;
    let dib_size = usize::try_from(u32_at(input, 14)?)
        .map_err(|_| invalid("BMP DIB size exceeds address space"))?;
    if dib_size < 40
        || 14_usize
            .checked_add(dib_size)
            .is_none_or(|header_end| header_end > offset || header_end > input.len())
    {
        return Err(invalid(
            "BMP needs a complete Windows DIB header before its pixels",
        ));
    }
    let signed_width = u32_at(input, 18)? as i32;
    let signed_height = u32_at(input, 22)? as i32;
    if signed_width <= 0 || signed_height == 0 || signed_height == i32::MIN {
        return Err(invalid(
            "BMP dimensions must be nonzero and width must be positive",
        ));
    }
    if u16_at(input, 26)? != 1 || u16_at(input, 28)? != 24 || u32_at(input, 30)? != 0 {
        return Err(invalid(
            "province BMP must have one plane, 24-bit BGR pixels and BI_RGB compression",
        ));
    }
    let width = signed_width as u32;
    let height = signed_height.unsigned_abs();
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(3))
        .ok_or_else(|| invalid("BMP row size overflows"))?;
    let stride = row_bytes
        .checked_add(3)
        .map(|bytes| bytes & !3)
        .ok_or_else(|| invalid("BMP row stride overflows"))?;
    let pixel_bytes = stride
        .checked_mul(height as usize)
        .ok_or_else(|| invalid("BMP pixel storage overflows"))?;
    let end = offset
        .checked_add(pixel_bytes)
        .ok_or_else(|| invalid("BMP pixel offset overflows"))?;
    if end > input.len() {
        return Err(invalid("truncated BMP pixel array"));
    }
    let declared_pixels = u32_at(input, 34)? as usize;
    if declared_pixels != 0 && declared_pixels != pixel_bytes {
        return Err(invalid(
            "BMP image size does not match its dimensions and padding",
        ));
    }
    let mut lookup = HashMap::with_capacity(definitions.len());
    for (&id, definition) in definitions {
        if id != definition.id {
            return Err(invalid("province definition key differs from its id"));
        }
        let [r, g, b] = definition.rgb;
        let color = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        if lookup.insert(color, id).is_some() {
            return Err(invalid("duplicate RGB in province definitions"));
        }
    }
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| invalid("BMP pixel count overflows"))?;
    let mut province_ids = Vec::new();
    province_ids
        .try_reserve_exact(count)
        .map_err(|_| invalid("province image exceeds available memory"))?;
    for y in 0..height as usize {
        let stored_y = if signed_height > 0 {
            height as usize - 1 - y
        } else {
            y
        };
        let row = &input[offset + stored_y * stride..offset + stored_y * stride + row_bytes];
        for (x, pixel) in row.chunks_exact(3).enumerate() {
            let color =
                (u32::from(pixel[2]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[0]);
            province_ids.push(*lookup.get(&color).ok_or_else(|| {
                invalid(format!(
                    "province BMP pixel ({x},{y}) has undefined RGB [{},{},{}]",
                    pixel[2], pixel[1], pixel[0]
                ))
            })?);
        }
    }
    Ok(ProvinceImage {
        width,
        height,
        province_ids,
    })
}

fn last_field<'a>(items: &'a [Item], key: &[u8]) -> Option<&'a Value> {
    items
        .iter()
        .rev()
        .find(|item| item.key.as_deref() == Some(key))
        .map(|item| &item.value)
}

/// Bare country-file colors and `rgb` use 0..255 components; `hsv` uses
/// normalized components. Quantization occurs here for display, not recording.
pub fn parse_color(value: &Value) -> io::Result<[u8; 3]> {
    let (hsv, items) = match value {
        Value::Block(items) => (false, items),
        Value::Tagged { tag, items } if tag.eq_ignore_ascii_case(b"rgb") => (false, items),
        Value::Tagged { tag, items } if tag.eq_ignore_ascii_case(b"hsv") => (true, items),
        _ => {
            return Err(invalid(
                "color must be an RGB triple or normalized HSV triple",
            ));
        }
    };
    if items.len() != 3 || items.iter().any(|item| item.key.is_some()) {
        return Err(invalid("color must have exactly three unkeyed components"));
    }
    let mut components = [0_f64; 3];
    for (component, item) in components.iter_mut().zip(items) {
        *component = item
            .value
            .scalar_str()
            .ok_or_else(|| invalid("color component is not a number"))?
            .parse()
            .map_err(|_| invalid("color component is not a number"))?;
        let limit = if hsv { 1.0 } else { 255.0 };
        if !component.is_finite() || !(0.0..=limit).contains(component) {
            return Err(invalid(format!("color component is outside 0..{limit}")));
        }
    }
    if hsv {
        let [h, s, v] = components;
        let hue = if h == 1.0 { 0.0 } else { h * 6.0 };
        let chroma = v * s;
        let x = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
        let rgb = match hue as u8 {
            0 => [chroma, x, 0.0],
            1 => [x, chroma, 0.0],
            2 => [0.0, chroma, x],
            3 => [0.0, x, chroma],
            4 => [x, 0.0, chroma],
            _ => [chroma, 0.0, x],
        };
        Ok(rgb.map(|channel| ((channel + v - chroma) * 255.0).round() as u8))
    } else {
        Ok(components.map(|component| component.round() as u8))
    }
}

fn country_file_color(input: &[u8]) -> io::Result<[u8; 3]> {
    let document = parser::parse(input)?;
    parse_color(
        last_field(&document.items, b"color")
            .ok_or_else(|| invalid("country file has no color"))?,
    )
}

fn apply_color_overrides(colors: &mut CountryColors, input: &[u8]) -> io::Result<()> {
    let document = parser::parse(input)?;
    for item in document.items {
        let Some(tag) = item.key else {
            return Err(invalid("colors.txt has an unnamed entry"));
        };
        let Some(items) = item.value.items() else {
            continue;
        };
        // color_ui has a different purpose and does not select political color.
        if let Some(color) = last_field(items, b"color") {
            colors.assign(tag, parse_color(color));
        }
    }
    Ok(())
}

pub fn read_country_colors(layers: &ResolvedLayers) -> io::Result<CountryColors> {
    let mut mappings = BTreeMap::new();
    for path in layers.paths().filter(|path| {
        path.to_ascii_lowercase()
            .starts_with("common/country_tags/")
            && path.to_ascii_lowercase().ends_with(".txt")
    }) {
        let document = parser::parse(&layers.read(path)?)?;
        for item in document.items {
            let Some(tag) = item.key else {
                return Err(invalid(format!("{path}: country tag entry has no key")));
            };
            if tag == b"dynamic_tags" {
                continue;
            }
            let relative = item
                .value
                .scalar_str()
                .ok_or_else(|| invalid(format!("{path}: country tag mapping is not a path")))?;
            let relative = normalize_relative(relative)?;
            if !relative.starts_with("countries/") {
                return Err(invalid(format!(
                    "{path}: country tag path is not under countries/"
                )));
            }
            let source = format!("common/{relative}");
            if mappings
                .insert(tag.clone(), source.clone())
                .is_some_and(|previous| previous != source)
            {
                return Err(invalid(format!(
                    "country tag {} has conflicting file declarations; precedence is not established",
                    String::from_utf8_lossy(&tag)
                )));
            }
        }
    }
    let mut colors = CountryColors::default();
    for (tag, path) in mappings {
        match layers.read(&path) {
            Ok(bytes) => colors.assign(
                tag,
                country_file_color(&bytes).map_err(|error| invalid(format!("{path}: {error}"))),
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => colors.assign(
                tag,
                Err(invalid(format!("mapped country file is missing: {path}"))),
            ),
            Err(error) => return Err(error),
        }
    }
    match layers.read("common/countries/colors.txt") {
        Ok(bytes) => apply_color_overrides(&mut colors, &bytes)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(colors)
}

pub fn load(layers: &ResolvedLayers) -> io::Result<MapData> {
    let definitions = parse_definitions(&layers.read("map/definition.csv")?)?;
    let image = decode_provinces_bmp(&layers.read("map/provinces.bmp")?, &definitions)?;
    Ok(MapData {
        width: image.width,
        height: image.height,
        province_ids: image.province_ids,
        definitions,
        country_colors: read_country_colors(layers)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(width: i32, height: i32, pixels: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0_u8; 54];
        bytes[..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&(54_u32 + pixels.len() as u32).to_le_bytes());
        bytes[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&width.to_le_bytes());
        bytes[22..26].copy_from_slice(&height.to_le_bytes());
        bytes[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&24_u16.to_le_bytes());
        bytes[34..38].copy_from_slice(&(pixels.len() as u32).to_le_bytes());
        bytes.extend_from_slice(pixels);
        bytes
    }

    #[test]
    fn bmp_bgr_padding_and_both_storage_directions_produce_top_left_ids() {
        let definitions = parse_definitions(b"0;0;0;0;land\n7;1;2;3;sea\n99;4;5;6;lake\n").unwrap();
        let top_row = [3, 2, 1, 6, 5, 4, 222, 222];
        let bottom_row = [0, 0, 0, 3, 2, 1, 111, 111];
        let expected = ProvinceImage {
            width: 2,
            height: 2,
            province_ids: vec![7, 99, 0, 7],
        };
        assert_eq!(
            decode_provinces_bmp(&bitmap(2, 2, &[bottom_row, top_row].concat()), &definitions)
                .unwrap(),
            expected
        );
        assert_eq!(
            decode_provinces_bmp(
                &bitmap(2, -2, &[top_row, bottom_row].concat()),
                &definitions
            )
            .unwrap(),
            expected
        );
    }

    #[test]
    fn bmp_unknown_pixels_truncation_and_unsupported_headers_are_named() {
        let definitions = parse_definitions(b"0;0;0;0;land\n").unwrap();
        let good = bitmap(1, 1, &[0, 0, 0, 88]);
        let unknown = bitmap(1, 1, &[3, 2, 1, 88]);
        assert!(
            decode_provinces_bmp(&unknown, &definitions)
                .unwrap_err()
                .to_string()
                .contains("undefined RGB [1,2,3]")
        );
        for length in 0..good.len() {
            assert!(decode_provinces_bmp(&good[..length], &definitions).is_err());
        }
        for (offset, value) in [
            (18, 0_u32),
            (22, i32::MIN as u32),
            (30, 1),
            (10, 40),
            (34, 999),
        ] {
            let mut bytes = good.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_provinces_bmp(&bytes, &definitions).is_err());
        }
        let mut wrong_bits = good;
        wrong_bits[28..30].copy_from_slice(&32_u16.to_le_bytes());
        assert!(decode_provinces_bmp(&wrong_bits, &definitions).is_err());
    }

    #[test]
    fn province_definitions_reject_aliasing_colors_duplicate_ids_and_unknown_kinds() {
        for invalid in [
            "1;1;2;3;land\n2;1;2;3;sea",
            "1;1;2;3;land\n1;3;2;1;sea",
            "1;256;0;0;land",
            "1;1;2;3;mountain",
            "1;1;2",
            "",
        ] {
            assert!(parse_definitions(invalid.as_bytes()).is_err());
        }
        let definitions =
            parse_definitions(b"\xef\xbb\xbf# header\n0;0;0;0;land;false;unknown;0\n").unwrap();
        assert_eq!(definitions[&0].kind, ProvinceKind::Land);
    }

    #[test]
    fn authored_overrides_choose_map_color_and_invalid_color_never_falls_back() {
        let mut colors = CountryColors::default();
        colors.assign(b"GER".to_vec(), country_file_color(b"color={106 119 89}"));
        colors.assign(b"ENG".to_vec(), country_file_color(b"color={1 2 3}"));
        apply_color_overrides(&mut colors, b"GER={color=HSV{0.1 0.15 0.4} color_ui=rgb{255 0 0}} ENG={color=rgb{201 56 93}} SOV={color=rgb{125 13 24}}").unwrap();
        assert_eq!(colors.rgb[b"GER".as_slice()], [102, 96, 87]);
        assert_eq!(colors.rgb[b"ENG".as_slice()], [201, 56, 93]);
        assert_eq!(colors.rgb[b"SOV".as_slice()], [125, 13, 24]);
        apply_color_overrides(&mut colors, b"ENG={color=HSV{2 1 1}}").unwrap();
        assert!(!colors.rgb.contains_key(b"ENG".as_slice()));
        assert!(colors.unavailable.contains_key(b"ENG".as_slice()));
        assert!(!colors.rgb.contains_key(b"XXX".as_slice()));
    }

    #[test]
    fn hsv_primaries_and_rgb_ranges_are_explicit() {
        for (input, expected) in [
            (b"color=hsv{0 1 1}".as_slice(), [255, 0, 0]),
            (b"color=HSV{1 1 1}", [255, 0, 0]),
            (b"color=hsv{0.5 1 1}", [0, 255, 255]),
            (b"color=hsv{0.2 0 0.5}", [128, 128, 128]),
        ] {
            assert_eq!(country_file_color(input).unwrap(), expected);
        }
        for invalid in [
            "color=rgb{256 0 0}",
            "color={-1 0 0}",
            "color=hsv{0 1.1 1}",
            "color=rgb{NaN 0 0}",
            "color={1 2}",
            "color={r=1 g=2 b=3}",
        ] {
            assert!(country_file_color(invalid.as_bytes()).is_err());
        }
    }
}
