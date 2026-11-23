use crate::invalid;
use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_map::map::{MapData, ProvinceKind};
use content_reader_parser::parser::{self, Value};
use content_reader_regions::regions::StrategicRegion;
use std::{collections::BTreeMap, io};

const SNOW_FIXED_POINT: f32 = 100_000.0;
pub const BAND_EPSILON: f32 = 0.001;

#[derive(Clone, Copy, Debug)]
pub struct SnowDefines {
    pub per_province: [f32; 2],
    pub total: [f32; 2],
}

pub trait Factors {
    fn snow_factors(&self, snow: &[Option<i64>]) -> io::Result<SnowFactors>;
    fn snow_records(&self) -> Option<&[Vec<SnowRecord>]> {
        None
    }
    fn snow_factor_at(
        &self,
        index: usize,
        snow: &map_display_data::Paged<Option<i64>>,
    ) -> io::Result<Option<f32>> {
        Ok(self.snow_factors_paged(snow)?[index])
    }
    fn snow_factors_paged(
        &self,
        snow: &map_display_data::Paged<Option<i64>>,
    ) -> io::Result<SnowFactors> {
        self.snow_factors(snow.try_as_slice()?)
    }
}

#[derive(Clone, Debug)]
pub struct Winter {
    pub defines: SnowDefines,
    pub records: Vec<Vec<SnowRecord>>,
}
impl Factors for Winter {
    fn snow_records(&self) -> Option<&[Vec<SnowRecord>]> {
        Some(&self.records)
    }
    fn snow_factor_at(
        &self,
        index: usize,
        snow: &map_display_data::Paged<Option<i64>>,
    ) -> io::Result<Option<f32>> {
        Ok(snow_factor_with(
            &self.records[index],
            |province| snow.get(province as usize).copied().flatten(),
            self.defines,
        ))
    }
    fn snow_factors_paged(
        &self,
        snow: &map_display_data::Paged<Option<i64>>,
    ) -> io::Result<SnowFactors> {
        SnowFactors::try_from_iter(
            self.records.len(),
            self.records.iter().map(|records| {
                snow_factor_with(
                    records,
                    |province| snow.get(province as usize).copied().flatten(),
                    self.defines,
                )
            }),
        )
    }
    fn snow_factors(&self, snow: &[Option<i64>]) -> io::Result<SnowFactors> {
        SnowFactors::try_from_iter(
            self.records.len(),
            self.records
                .iter()
                .map(|records| snow_factor(records, snow, self.defines)),
        )
    }
}
pub fn read_post_effects(layers: &ResolvedLayers) -> io::Result<parser::Document> {
    let file = "gfx/posteffect_volumes.txt";
    parser::parse_unclosed(&layers.read(file)?)
        .map(|(document, _)| document)
        .map_err(|error| invalid(format!("{file}: {error}")))
}

impl Winter {
    pub fn load(
        layers: &ResolvedLayers,
        defines: &Defines,
        map: &MapData,
        regions: &BTreeMap<u32, StrategicRegion>,
    ) -> io::Result<Self> {
        let document = read_post_effects(layers)?;
        let mut names = std::collections::BTreeSet::new();
        for value in document.get_all("posteffect_values") {
            let value = as_document(value).ok_or_else(|| {
                invalid("gfx/posteffect_volumes.txt: posteffect_values is not a block")
            })?;
            names.insert(scalar(&value, "name").ok_or_else(|| {
                invalid("gfx/posteffect_volumes.txt: posteffect_values without a name")
            })?);
        }
        let (_, boxes) = volumes(&document, |name| names.contains(name))?;
        let extents = province_extents(map);
        let number = |key: &str| -> io::Result<f32> {
            let value = defines.number("NGraphics", key)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!("NDefines.NGraphics.{key} is outside f32")))
            }
        };
        Ok(Self {
            defines: SnowDefines {
                per_province: [
                    number("POSTEFFECT_PER_PROVINCE_MIN_SNOW")?,
                    number("POSTEFFECT_PER_PROVINCE_MAX_SNOW")?,
                ],
                total: [
                    number("POSTEFFECT_TOTAL_MIN_SNOW")?,
                    number("POSTEFFECT_TOTAL_MAX_SNOW")?,
                ],
            },
            records: boxes
                .iter()
                .map(|volume| {
                    snow_records(
                        volume.ground_bounds(),
                        regions.values().map(|r| r.provinces.as_slice()),
                        &extents,
                    )
                })
                .collect(),
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProvinceExtent {
    pub min: [i32; 2],
    pub size: [i32; 2],
    pub area: i32,
    pub land: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnowRecord {
    pub province: u32,
    pub weight: f32,
}

pub use map_display_data::winter::SnowFactors;

#[derive(Clone, Debug)]
pub struct BoxVolume {
    pub day: String,
    pub night: String,
    pub day_winter: Option<String>,
    pub night_winter: Option<String>,
    pub position: [f32; 3],
    pub size: [f32; 3],
    pub fade_distance: f32,
    pub records: Vec<SnowRecord>,
}

impl BoxVolume {
    pub fn ground_bounds(&self) -> [f32; 4] {
        [
            self.position[0] - self.size[0] * 0.5,
            self.position[2] - self.size[2] * 0.5,
            self.position[0] + self.size[0] * 0.5,
            self.position[2] + self.size[2] * 0.5,
        ]
    }
}

pub fn saturate(value: f32) -> f32 {
    if 0.0 <= value { value.min(1.0) } else { 0.0 }
}

pub fn band(value: f32, low: f32, high: f32) -> f32 {
    let width = if high - low <= BAND_EPSILON {
        BAND_EPSILON
    } else {
        high - low
    };
    saturate((value - low) / width)
}

pub fn province_extents(map: &MapData) -> Vec<Option<ProvinceExtent>> {
    let len = map
        .definitions
        .keys()
        .copied()
        .chain(map.province_ids.iter().copied())
        .max()
        .map_or(0, |id| id as usize + 1);
    let mut extents: Vec<Option<ProvinceExtent>> = vec![None; len];
    for (&id, definition) in &map.definitions {
        extents[id as usize] = Some(ProvinceExtent {
            land: definition.kind == ProvinceKind::Land,
            ..ProvinceExtent::default()
        });
    }
    let mut bounds = vec![[i32::MAX, i32::MAX, -1, -1]; len];
    let width = map.width as usize;
    let height = map.height as i32;
    for (index, &id) in map.province_ids.iter().enumerate() {
        let column = (index % width.max(1)) as i32;
        let row = height - 1 - (index / width.max(1)) as i32;
        let Some(Some(extent)) = extents.get_mut(id as usize) else {
            continue;
        };
        extent.area = extent.area.wrapping_add(1);
        let b = &mut bounds[id as usize];
        b[0] = b[0].min(column);
        b[1] = b[1].min(row);
        b[2] = b[2].max(column);
        b[3] = b[3].max(row);
    }
    for (extent, b) in extents.iter_mut().zip(&bounds) {
        if let Some(extent) = extent
            && b[2] >= 0
        {
            extent.min = [b[0] & !1, b[1] & !1];
            extent.size = [
                ((b[2] + 2) & !1) - extent.min[0],
                ((b[3] + 2) & !1) - extent.min[1],
            ];
        }
    }
    extents
}

pub fn snow_records<'a>(
    bounds: [f32; 4],
    regions: impl IntoIterator<Item = &'a [u32]>,
    extents: &[Option<ProvinceExtent>],
) -> Vec<SnowRecord> {
    let [min_x, min_z, max_x, max_z] = bounds;
    let mut areas = Vec::new();
    let mut total = 0u32;
    let mut seen = Vec::new();
    for provinces in regions {
        seen.clear();
        let mut area = 0i32;
        let mut representative = None;
        for &id in provinces {
            let Some(Some(extent)) = extents.get(id as usize) else {
                continue;
            };
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let overlaps = (extent.min[0] as f32) < max_x
                && min_x < extent.min[0].wrapping_add(extent.size[0]) as f32
                && (extent.min[1] as f32) < max_z
                && min_z < extent.min[1].wrapping_add(extent.size[1]) as f32;
            if extent.land && overlaps {
                representative = Some(id);
                area = area.wrapping_add(extent.area);
            }
        }
        if let (true, Some(province)) = (area != 0, representative) {
            areas.push((province, area));
            total = total.wrapping_add(area as u32);
        }
    }
    areas
        .into_iter()
        .map(|(province, area)| SnowRecord {
            province,
            weight: saturate(area as f32 / total as i32 as f32),
        })
        .collect()
}

pub fn snow_factor(
    records: &[SnowRecord],
    snow: &[Option<i64>],
    defines: SnowDefines,
) -> Option<f32> {
    snow_factor_with(
        records,
        |province| snow.get(province as usize).copied().flatten(),
        defines,
    )
}

pub fn snow_factor_with(
    records: &[SnowRecord],
    snow: impl Fn(u32) -> Option<i64>,
    defines: SnowDefines,
) -> Option<f32> {
    let mut sum = 0.0f32;
    for record in records {
        let raw = snow(record.province)?;
        let province = band(
            raw as f32 / SNOW_FIXED_POINT,
            defines.per_province[0],
            defines.per_province[1],
        );
        sum += province * record.weight;
    }
    Some(band(sum, defines.total[0], defines.total[1]))
}

fn as_document(value: &Value) -> Option<parser::Document> {
    Some(parser::Document {
        items: value.items()?.to_vec(),
    })
}

fn scalar(document: &parser::Document, key: &str) -> Option<String> {
    document.get(key).and_then(Value::scalar_str).map(unquote)
}

fn number(document: &parser::Document, key: &str) -> Result<Option<f32>, String> {
    let Some(value) = document.get(key) else {
        return Ok(None);
    };
    value
        .scalar_str()
        .and_then(|text| text.trim().parse().ok())
        .map(Some)
        .ok_or_else(|| format!("{key} is not a number"))
}

fn triple(document: &parser::Document, key: &str) -> Result<[f32; 3], String> {
    let items = document
        .get(key)
        .and_then(Value::items)
        .filter(|items| items.len() == 3)
        .ok_or_else(|| format!("{key} is not three numbers"))?;
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item
            .value
            .scalar_str()
            .and_then(|text| text.trim().parse().ok())
            .ok_or_else(|| format!("{key} is not three numbers"))?;
    }
    Ok(out)
}

fn unquote(text: &str) -> String {
    text.trim().trim_matches('"').to_string()
}

#[derive(Clone, Debug)]
pub struct HeightVolume {
    pub day: String,
    pub night: String,
    pub height: f32,
    pub fade_distance: f32,
}

pub fn volumes(
    document: &parser::Document,
    known: impl Fn(&str) -> bool,
) -> io::Result<(Vec<HeightVolume>, Vec<BoxVolume>)> {
    let refuse = |error: String| invalid(format!("gfx/posteffect_volumes.txt: {error}"));
    let named =
        |volume: &parser::Document, key: &str| scalar(volume, key).filter(|name| known(name));
    let mut heights = Vec::new();
    let mut boxes = Vec::new();
    for outer in document.get_all("posteffect_volumes") {
        let outer =
            as_document(outer).ok_or_else(|| refuse("posteffect_volumes is not a block".into()))?;
        for value in outer.get_all("posteffect_height_volume") {
            let volume = as_document(value)
                .ok_or_else(|| refuse("posteffect_height_volume is not a block".into()))?;
            let (Some(day), Some(night)) = (
                named(&volume, "posteffect_values_day"),
                named(&volume, "posteffect_values_night"),
            ) else {
                continue;
            };
            heights.push(HeightVolume {
                day,
                night,
                height: number(&volume, "height").map_err(refuse)?.unwrap_or(0.0),
                fade_distance: number(&volume, "fade_distance")
                    .map_err(refuse)?
                    .unwrap_or(0.0),
            });
        }
        for value in outer.get_all("posteffect_volume") {
            let volume = as_document(value)
                .ok_or_else(|| refuse("posteffect_volume is not a block".into()))?;
            let (Some(day), Some(night)) = (
                named(&volume, "posteffect_values_day"),
                named(&volume, "posteffect_values_night"),
            ) else {
                continue;
            };
            boxes.push(BoxVolume {
                day,
                night,
                day_winter: named(&volume, "posteffect_values_day_winter"),
                night_winter: named(&volume, "posteffect_values_night_winter"),
                position: triple(&volume, "position").map_err(refuse)?,
                size: triple(&volume, "size").map_err(refuse)?,
                fade_distance: number(&volume, "fade_distance")
                    .map_err(refuse)?
                    .unwrap_or(0.0),
                records: Vec::new(),
            });
        }
    }
    Ok((heights, boxes))
}
