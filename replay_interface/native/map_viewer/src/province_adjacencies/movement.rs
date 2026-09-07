use super::{native_line, ProvinceAdjacencies};
use crate::authored_fixed::parse_fixed5;
use crate::mod_resolver::GameFileResolver;
use crate::province_positions::{engine_province_anchors, engine_records_and_raster, for_each_run, EngineMap};
use crate::ContentAdmissionLease;
use portable_content_manifest::paradox_text::{self, Value};
use portable_content_manifest::UsedContentKey;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvinceMovementDistance {
    Exact(i64),
    Unavailable(ProvinceMovementDistanceUnavailable),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvinceMovementDistanceUnavailable {
    Content,
    DefaultMap,
    TerrainBitmap,
    TerrainDefinitions,
    TerrainNamespaceUnwitnessed,
    TerrainCategoryIdentity,
    TerrainPixelUnwritten,
    OriginalCentre,
    NativePathCapacity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceMovementContent {
    /// Default map, selected bitmap, then terrain winners in native load order.
    pub sources: Vec<UsedContentKey>,
    /// Partial keys remain diagnostic; only Ok certifies the complete input census.
    pub availability: Result<(), ProvinceMovementDistanceUnavailable>,
}

type Missing = ProvinceMovementDistanceUnavailable;

pub(super) fn populate(
    table: &mut ProvinceAdjacencies,
    map: &EngineMap,
    resolver: &GameFileResolver,
) {
    let mut sources = Vec::new();
    let inputs = Inputs::load(resolver, &mut sources);
    table.movement_content = ProvinceMovementContent {
        sources, availability: inputs.as_ref().map_err(|reason| *reason)
            .and_then(|inputs| inputs.movement_error.map_or(Ok(()), Err)),
    };
    let anchors = engine_province_anchors(&map.ids, map.width, map.height, &map.defined);
    let mut distances = BTreeMap::new();
    for (&source, entries) in &table.entries {
        for edge in entries {
            if source > edge.target_province_id { continue; }
            let value = inputs.as_ref().map_err(|reason| *reason).and_then(|inputs| {
                let a = anchors.get(&source).ok_or(Missing::OriginalCentre)?;
                let b = anchors.get(&edge.target_province_id).ok_or(Missing::OriginalCentre)?;
                inputs.distance(
                    [a.engine_x, a.engine_y], [b.engine_x, b.engine_y], edge.crossing,
                    map.width, map.height,
                )
            });
            distances.insert((source, edge.target_province_id), match value {
                Ok(raw) => ProvinceMovementDistance::Exact(raw),
                Err(reason) => ProvinceMovementDistance::Unavailable(reason),
            });
        }
    }
    for (&source, entries) in &mut table.entries {
        for edge in entries {
            edge.movement_distance_raw = distances[&(source.min(edge.target_province_id), source.max(edge.target_province_id))];
        }
    }
}

struct Inputs {
    terrain: Terrain,
    costs: Vec<std::result::Result<i64, Missing>>,
    snow: Vec<bool>,
    movement_error: Option<Missing>,
}

impl Inputs {
    fn load(resolver: &GameFileResolver, sources: &mut Vec<UsedContentKey>) -> std::result::Result<Self, Missing> {
        Self::load_with_map(resolver, sources).map(|(inputs, _)| inputs)
    }

    fn load_with_map(resolver: &GameFileResolver, sources: &mut Vec<UsedContentKey>) -> Result<(Self, Value), Missing> {
        let bytes = read_source(resolver, "map/default.map", sources)?;
        let root = document(&bytes)?;
        let mut terrain_path = None;
        for field in root.fields().iter().filter(|field| field.key_matches("terrain")) {
            terrain_path = Some(field.value.as_scalar().ok_or(Missing::DefaultMap)?);
        }
        let path = format!("map/{}", terrain_path.ok_or(Missing::DefaultMap)?);
        let terrain = Terrain::decode(&read_source(resolver, &path, sources)?)?;
        resolver.require_complete_admitted_dir("common/terrain", Some("txt"), false, "province movement terrain")
            .map_err(|_| Missing::TerrainNamespaceUnwitnessed)?;
        let files = resolver.resolve_dir_files("common/terrain", Some("txt"))
            .map_err(|_| Missing::Content)?;
        let mut categories = Vec::new();
        let mut costs = Vec::new();
        let mut snow = Vec::new();
        let mut movement_error = None;
        for (path, _) in files {
            let bytes = read_source(resolver, &path, sources)?;
            let root = document(&bytes)?;
            append_definitions(&root, &mut categories, &mut costs, &mut snow, &mut movement_error)?;
        }
        Ok((Self { terrain, costs, snow, movement_error }, root))
    }

    fn distance(
        &self, a: [i32; 2], mut b: [i32; 2], crossing: [i32; 2], width: usize, height: usize,
    ) -> std::result::Result<i64, Missing> {
        if let Some(reason) = self.movement_error { return Err(reason); }
        let width = i32::try_from(width).map_err(|_| Missing::NativePathCapacity)?;
        let capacity = (width as usize).checked_add(height).ok_or(Missing::NativePathCapacity)?;
        if b[0].wrapping_sub(a[0]).wrapping_abs() > width / 2 {
            b[0] = b[0].wrapping_add(if a[0] < width / 2 { -width } else { width });
        }
        let mut points = native_line(a, crossing, capacity);
        if points.is_empty() { return Err(Missing::NativePathCapacity); }
        // The engine swaps only the end points, not the segment's interior order.
        if points[0] == crossing {
            let last = points.len() - 1;
            points.swap(0, last);
        }
        points.pop();
        points.extend(native_line(crossing, b, capacity));
        if points.len() > capacity {
            return Err(Missing::NativePathCapacity);
        }
        let mut sum = 0i64;
        for [x, y] in points {
            let index = self.terrain.index(x.wrapping_add(width) % width, y)?;
            if let Some(cost) = self.costs.get(index as usize) {
                sum = sum.wrapping_add((*cost)?);
            }
        }
        Ok(native_distance_scale(sum))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermanentSnowUnavailable {
    ContentAdmission,
    Terrain(ProvinceMovementDistanceUnavailable),
    ProvinceMap,
    UnknownProvince,
    InactiveProvince,
    NativePixelCountFault,
    NonPositivePixelCount,
    MaximumUnobtained,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PermanentSnowCoverage {
    pixel_count: u32,
    snow_pixels: u32,
}

impl PermanentSnowCoverage {
    pub fn pixel_count(self) -> u32 { self.pixel_count }
    pub fn snow_pixels(self) -> u32 { self.snow_pixels }

    /// Initial +30 only. Current province/row validity and the recorded maximum remain caller obligations.
    pub fn raw_at_observed_max(self, maximum: Option<i64>) -> Result<i64, PermanentSnowUnavailable> {
        let maximum = maximum.ok_or(PermanentSnowUnavailable::MaximumUnobtained)?;
        let pixels = i64::from(self.pixel_count as i32);
        if pixels <= 0 { return Err(PermanentSnowUnavailable::NonPositivePixelCount); }
        // 140f088a7..8945 sign-extends both counters; each IMUL truncates before the signed division.
        let numerator = i64::from(self.snow_pixels as i32).wrapping_mul(100_000);
        let ratio = numerator.wrapping_mul(100_000) / pixels.wrapping_mul(100_000);
        Ok((maximum.wrapping_mul(50_000) / 100_000).wrapping_mul(ratio) / 100_000)
    }
}

pub struct ProvincePermanentSnow {
    lease: ContentAdmissionLease,
    sources: Vec<UsedContentKey>,
    provinces: BTreeMap<u32, Result<PermanentSnowCoverage, PermanentSnowUnavailable>>,
}

impl ProvincePermanentSnow {
    pub fn load(resolver: &GameFileResolver) -> Result<Self, PermanentSnowUnavailable> {
        let lease = resolver.content_admission_lease().ok_or(PermanentSnowUnavailable::ContentAdmission)?;
        let mut sources = Vec::new();
        let (inputs, root) = Inputs::load_with_map(resolver, &mut sources).map_err(PermanentSnowUnavailable::Terrain)?;
        let path = |key: &str| -> Result<String, PermanentSnowUnavailable> {
            let mut value = None;
            for field in root.fields().iter().filter(|field| field.key_matches(key)) {
                value = Some(field.value.as_scalar().ok_or(PermanentSnowUnavailable::ProvinceMap)?);
            }
            let value = value.ok_or(PermanentSnowUnavailable::ProvinceMap)?;
            if value.is_empty() || value.starts_with('/') || value.contains('\\')
                || value.split('/').any(|part| matches!(part, "" | "." | "..")) {
                return Err(PermanentSnowUnavailable::ProvinceMap);
            }
            Ok(format!("map/{value}"))
        };
        let definitions = read_source(resolver, &path("definitions")?, &mut sources).map_err(PermanentSnowUnavailable::Terrain)?;
        let bitmap = read_source(resolver, &path("provinces")?, &mut sources).map_err(PermanentSnowUnavailable::Terrain)?;
        let map = engine_records_and_raster(&definitions, &bitmap).map_err(|_| PermanentSnowUnavailable::ProvinceMap)?;
        let provinces = inputs.snow_coverage(&map);
        Ok(Self { lease, sources, provinces })
    }

    pub fn admission_lease(&self) -> &ContentAdmissionLease { &self.lease }
    pub fn sources(&self) -> &[UsedContentKey] { &self.sources }
    pub fn province(&self, id: u32) -> Result<PermanentSnowCoverage, PermanentSnowUnavailable> {
        self.provinces.get(&id).copied().unwrap_or(Err(PermanentSnowUnavailable::UnknownProvince))
    }
}

impl Inputs {
    fn snow_coverage(&self, map: &EngineMap) -> BTreeMap<u32, Result<PermanentSnowCoverage, PermanentSnowUnavailable>> {
        let mut counts: BTreeMap<_, _> = map.defined.iter().map(|id| (*id,
            (false, PermanentSnowCoverage { pixel_count: 0, snow_pixels: 0 }, None))).collect();
        for_each_run(&map.ids, map.width, map.height, |id, y, start, length| {
            let Some((has_run, coverage, refusal)) = counts.get_mut(&id) else { return; };
            *has_run = true;
            coverage.pixel_count = coverage.pixel_count.wrapping_add(length as u32);
            for pixel in 0..length {
                match self.terrain.index(start.wrapping_add(pixel), y) {
                    Ok(index) if self.snow.get(index as usize).copied().unwrap_or(false) => {
                        coverage.snow_pixels = coverage.snow_pixels.wrapping_add(1);
                    }
                    Ok(_) => {}
                    Err(reason) => { *refusal = Some(PermanentSnowUnavailable::Terrain(reason)); }
                }
            }
        });
        counts.into_iter().map(|(id, (has_run, coverage, refusal))| {
            let value = if !has_run { Err(PermanentSnowUnavailable::InactiveProvince) }
                else if coverage.pixel_count == 0 { Err(PermanentSnowUnavailable::NativePixelCountFault) }
                else if coverage.pixel_count as i32 <= 0 { Err(PermanentSnowUnavailable::NonPositivePixelCount) }
                else if let Some(reason) = refusal { Err(reason) }
                else { Ok(coverage) };
            (id, value)
        }).collect()
    }
}

fn read_source(
    resolver: &GameFileResolver, path: &str, sources: &mut Vec<UsedContentKey>,
) -> Result<Vec<u8>, Missing> {
    let key = resolver.admitted_used_content_key(path).map_err(|_| Missing::Content)?
        .ok_or(Missing::Content)?;
    sources.push(key.clone());
    resolver.read_admitted_used_content(&key).map_err(|_| Missing::Content)
}

fn native_distance_scale(sum: i64) -> i64 {
    if (sum as u64).wrapping_add(0x7fff_ffff_ffff_fffe) <= 0xffff_ffff_ffff_fffc {
        sum.wrapping_mul(100_000) / 300_000
    } else {
        0xffff_ffff
    }
}

fn document(bytes: &[u8]) -> std::result::Result<Value, Missing> {
    let text = paradox_text::decode_text_strict(bytes).map_err(|_| Missing::TerrainDefinitions)?;
    let mut root = paradox_text::parse_str_strict(&text).map_err(|_| Missing::TerrainDefinitions)?;
    paradox_text::expand_file_scoped_variables(&mut root).map_err(|_| Missing::TerrainDefinitions)?;
    Ok(root)
}

fn append_definitions(
    root: &Value,
    categories: &mut Vec<(String, std::result::Result<i64, Missing>)>,
    costs: &mut Vec<std::result::Result<i64, Missing>>,
    snow: &mut Vec<bool>,
    movement_error: &mut Option<Missing>,
) -> std::result::Result<(), Missing> {
    for container in root.fields() {
        if !container.key_is_quoted && container.key_matches("categories") {
            let Some(entries) = container.value.as_object() else {
                *movement_error = Some(Missing::TerrainDefinitions);
                continue;
            };
            for category in entries {
                let Some(name) = category.key.as_ref() else {
                    *movement_error = Some(Missing::TerrainDefinitions);
                    continue;
                };
                if name.starts_with('@') { continue; }
                let Some(fields) = category.value.as_object() else {
                    categories.push((name.clone(), Err(Missing::TerrainDefinitions)));
                    continue;
                };
                let mut cost = Ok(0);
                for field in fields {
                    if field.key_matches("movement_cost") {
                        cost = field.value.as_scalar().and_then(parse_fixed5)
                            .map(|raw| raw.max(1000)).ok_or(Missing::TerrainDefinitions);
                    }
                }
                categories.push((name.clone(), cost));
            }
        } else if !container.key_is_quoted && container.key_matches("terrain") {
            for definition in container.value.as_object().ok_or(Missing::TerrainDefinitions)? {
                if definition.key.as_deref().is_some_and(|name| name.starts_with('@')) { continue; }
                let mut cost = Ok(0);
                let mut colors = Vec::new();
                let mut permanent = false;
                for field in definition.value.as_object().ok_or(Missing::TerrainDefinitions)? {
                    if !field.key_is_quoted && field.key_matches("type") {
                        let Some(name) = field.value.as_scalar() else {
                            cost = Err(Missing::TerrainDefinitions);
                            continue;
                        };
                        cost = match categories.iter().find(|(candidate, _)| candidate.eq_ignore_ascii_case(name)) {
                            Some((candidate, value)) if candidate == name => *value,
                            Some(_) => Err(Missing::TerrainCategoryIdentity),
                            None => Ok(0),
                        };
                    } else if !field.key_is_quoted && field.key_matches("perm_snow") {
                        // 1424b06e0 writes only exact yes/no; invalid text retains the constructor/prior byte.
                        match field.value.as_scalar() {
                            Some("yes") => permanent = true,
                            Some("no") => permanent = false,
                            _ => {}
                        }
                    } else if !field.key_is_quoted && field.key_matches("color") {
                        for item in field.value.as_object().ok_or(Missing::TerrainDefinitions)? {
                            let index = item.value.as_scalar().and_then(|s| s.parse::<i32>().ok())
                                .filter(|index| *index >= 0).ok_or(Missing::TerrainDefinitions)?;
                            colors.push(index as usize);
                        }
                    }
                }
                for index in colors {
                    // Only byte indices are consumed; larger indices still extend the witnessed census.
                    costs.resize(costs.len().max(index.saturating_add(1).min(256)), Ok(0));
                    snow.resize(snow.len().max(index.saturating_add(1).min(256)), false);
                    if let Some(slot) = costs.get_mut(index) { *slot = cost; }
                    if let Some(slot) = snow.get_mut(index) { *slot = permanent; }
                }
            }
        }
    }
    Ok(())
}


struct Terrain {
    width: i32,
    height: i32,
    components: usize,
    pitch: usize,
    bytes: Vec<u8>,
}

impl Terrain {
    fn decode(data: &[u8]) -> std::result::Result<Self, Missing> {
        let head = data.get(..54).ok_or(Missing::TerrainBitmap)?;
        if &head[..2] != b"BM" { return Err(Missing::TerrainBitmap); }
        let word = |at| i32::from_le_bytes(head[at..at + 4].try_into().unwrap());
        let width = word(18);
        let height = word(22);
        let bpp = u16::from_le_bytes(head[28..30].try_into().unwrap());
        let components = match bpp { 1 | 8 => 1usize, 16 => 2, 24 => 3, _ => return Err(Missing::TerrainBitmap) };
        if width <= 0 || height <= 0 || word(30) != 0 { return Err(Missing::TerrainBitmap); }
        let row = i64::from(width) * components as i64;
        let pitch = if bpp == 1 { row >> 3 } else { row };
        let padded = if bpp == 1 { (row + 3) >> 3 } else { (row + 3) & !3 };
        let advance = if pitch < 1 { padded } else { pitch };
        let allocation = pitch * i64::from(height) + 7;
        let written = advance * i64::from(height - 1) + padded;
        if allocation > i64::from(i32::MAX) || allocation < written || advance < 1 {
            return Err(Missing::TerrainBitmap);
        }
        let start = if bpp == 24 { 54 } else { u32::from_le_bytes(head[10..14].try_into().unwrap()) as usize };
        let end = start.checked_add(padded as usize * height as usize).ok_or(Missing::TerrainBitmap)?;
        if end > data.len() { return Err(Missing::TerrainBitmap); }
        let mut bytes = vec![0; written as usize];
        for y in 0..height as usize {
            bytes[y * advance as usize..y * advance as usize + padded as usize]
                .copy_from_slice(&data[start + y * padded as usize..start + (y + 1) * padded as usize]);
        }
        Ok(Self { width, height, components, pitch: advance as usize, bytes })
    }

    fn index(&self, x: i32, y: i32) -> std::result::Result<u8, Missing> {
        if x < 0 || y < 0 { return Err(Missing::TerrainPixelUnwritten); }
        if x >= self.width || y >= self.height { return Ok(0); }
        self.bytes.get(y as usize * self.pitch + x as usize * self.components)
            .copied().ok_or(Missing::TerrainPixelUnwritten)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(source: &str) -> Vec<std::result::Result<i64, Missing>> {
        let mut categories = Vec::new();
        let mut costs = Vec::new();
        append_definitions(&document(source.as_bytes()).unwrap(), &mut categories, &mut costs, &mut Vec::new(), &mut None).unwrap();
        costs
    }

    #[test]
    fn palette_captures_current_first_category_and_last_index_writer() {
        let costs = palette(r#"
            categories = { first = { movement_cost = 1 } first = { movement_cost = 9 } }
            terrain = { a = { type = first color = { 2 } } early = { type = later color = { 1 } } }
            categories = { later = { movement_cost = 3 } }
            terrain = { b = { type = later color = { 2 4 } } }
        "#);
        assert_eq!(costs, vec![Ok(0), Ok(0), Ok(300_000), Ok(0), Ok(300_000)]);
        assert_eq!(palette("categories={ x={ movement_cost=1 } x={ movement_cost=9 }} terrain={ a={type=x color={0}} }"), [Ok(100_000)]);
    }

    #[test]
    fn terrain_weights_use_the_shared_native_auto_base_prefix_parser() {
        let root = document(br#"categories={
            octal={movement_cost=010} hex={movement_cost=0x10}
            suffix={movement_cost=1junk.25} signed={movement_cost=1.-2}
            clamped={movement_cost=08} invalid={movement_cost=bad}
        } terrain={
            a={type=octal color={0}} b={type=hex color={1}}
            c={type=suffix color={2}} d={type=signed color={3}}
            e={type=clamped color={4}} f={type=invalid color={5}}
        }"#).unwrap();
        let mut categories = Vec::new();
        let mut costs = Vec::new();
        append_definitions(&root, &mut categories, &mut costs, &mut Vec::new(), &mut None).unwrap();
        assert_eq!(costs, [Ok(800_000), Ok(1_600_000), Ok(125_000), Ok(98_000),
            Ok(1_000), Err(Missing::TerrainDefinitions)]);
    }

    #[test]
    fn omitted_member_zero_is_distinct_from_explicit_clamped_zero() {
        assert_eq!(palette(r#"
            categories = { omitted = {} explicit = { movement_cost = 0 } negative = { movement_cost = -1.234567 } }
            terrain = { a = { type = omitted color = { 0 } } b = { type = explicit color = { 1 } }
                c = { type = negative color = { 2 } } d = { color = { 3 } } }
        "#), [Ok(0), Ok(1000), Ok(1000), Ok(0)]);
        assert_eq!(parse_fixed5("-0.1234567"), Some(-12_345));
        assert_eq!(parse_fixed5("92233720368547.75807"), Some(i64::MAX));
        assert_eq!(parse_fixed5("-92233720368547.75808"), Some(i64::MIN));
    }

    #[test]
    fn unavailable_category_weight_does_not_poison_unrelated_palette_entries() {
        let costs = palette("categories={ bad={movement_cost=broken} good={movement_cost=2}} terrain={a={type=bad color={1}} b={type=good color={2}}}");
        assert_eq!(costs, [Ok(0), Err(Missing::TerrainDefinitions), Ok(200_000)]);
        assert_eq!(palette("categories={ X={movement_cost=2}} terrain={a={type=x color={1}}}")[1], Err(Missing::TerrainCategoryIdentity));
    }

    #[test]
    fn raw_distance_uses_wrapping_multiply_signed_division_and_zero_extended_sentinel() {
        assert_eq!(native_distance_scale(700_000), 233_333);
        assert_eq!(native_distance_scale(-700_000), -233_333);
        for raw in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(native_distance_scale(raw), 4_294_967_295);
        }
        assert_eq!(native_distance_scale(i64::MIN + 2), 0);
        assert_eq!(native_distance_scale(i64::MAX - 1), 0);
        assert_eq!(native_distance_scale(200_000_000_000_000), 5_177_519_754_301);
    }

    fn terrain(rows: &[[u8; 4]]) -> Terrain {
        let mut data = vec![0; 54 + rows.len() * 4];
        data[..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&54u32.to_le_bytes());
        data[18..22].copy_from_slice(&4i32.to_le_bytes());
        data[22..26].copy_from_slice(&(rows.len() as i32).to_le_bytes());
        data[28..30].copy_from_slice(&8u16.to_le_bytes());
        for (row, values) in rows.iter().enumerate() {
            data[54 + row * 4..58 + row * 4].copy_from_slice(values);
        }
        Terrain::decode(&data).unwrap()
    }

    #[test]
    fn weighted_route_uses_file_rows_crossing_once_and_only_consumed_costs() {
        let input = Inputs { terrain: terrain(&[[0, 0, 2, 0], [0, 1, 0, 0]]),
            costs: vec![Ok(100_000), Ok(200_000), Ok(400_000), Err(Missing::TerrainDefinitions)], snow: vec![], movement_error: None };
        assert_eq!(input.distance([0, 0], [2, 0], [1, 1], 4, 2), Ok(233_333));
        let input = Inputs { terrain: terrain(&[[0, 0, 2, 0], [0, 3, 0, 0]]), ..input };
        assert_eq!(input.distance([0, 0], [2, 0], [1, 1], 4, 2), Err(Missing::TerrainDefinitions));
    }

    #[test]
    fn wrapped_path_and_unassigned_palette_slots_preserve_exact_zero() {
        let input = Inputs { terrain: terrain(&[[0, 1, 1, 2], [0, 0, 0, 0]]),
            costs: vec![Ok(100_000), Ok(9_000_000), Ok(200_000)], snow: vec![], movement_error: None };
        assert_eq!(input.distance([0, 0], [3, 0], [0, 0], 4, 2), Ok(100_000));
        let input = Inputs { costs: vec![], ..input };
        assert_eq!(input.distance([0, 0], [3, 0], [0, 0], 4, 2), Ok(0));
    }

    #[test]
    fn terrain_uses_first_component_and_refuses_unwritten_bytes() {
        let mut data = vec![0; 58];
        data[..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&54u32.to_le_bytes());
        data[18..22].copy_from_slice(&2i32.to_le_bytes());
        data[22..26].copy_from_slice(&1i32.to_le_bytes());
        data[28..30].copy_from_slice(&16u16.to_le_bytes());
        data[54..].copy_from_slice(&[7, 99, 8, 77]);
        let bitmap = Terrain::decode(&data).unwrap();
        assert_eq!(bitmap.index(0, 0), Ok(7));
        assert_eq!(bitmap.index(1, 0), Ok(8));
        assert_eq!(bitmap.index(2, 0), Ok(0));
        data[28..30].copy_from_slice(&1u16.to_le_bytes());
        data[18..22].copy_from_slice(&16i32.to_le_bytes());
        let bitmap = Terrain::decode(&data).unwrap();
        assert_eq!(bitmap.index(7, 0), Err(Missing::TerrainPixelUnwritten));
        data[22..26].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(matches!(Terrain::decode(&data), Err(Missing::TerrainBitmap)));
    }

    fn snow_inputs(source: &str, terrain: Terrain) -> Inputs {
        let mut categories = Vec::new();
        let mut costs = Vec::new();
        let mut snow = Vec::new();
        let mut movement_error = None;
        append_definitions(&document(source.as_bytes()).unwrap(), &mut categories,
            &mut costs, &mut snow, &mut movement_error).unwrap();
        Inputs { terrain, costs, snow, movement_error }
    }

    #[test]
    fn permanent_snow_preserves_boolean_no_write_and_last_graphics_color_writer() {
        let input = snow_inputs(r#"
            terrain={
                first={perm_snow=yes color={1 2}}
                reset={perm_snow=yes perm_snow=no color={2}}
                invalid={perm_snow=yes perm_snow=YES perm_snow=unknown color={3}}
                quoted_value={perm_snow="yes" color={4}}
                quoted_key={"perm_snow"=yes color={5}}
                default_value={color={6}}
                mixed={PeRm_SnOw=yes color={7}}
                false_value={perm_snow=no color={8}}
                ignored_palette_key={perm_snow=no "color"={1}}
            }
            "terrain"={ignored={perm_snow=yes color={6}}}
        "#, terrain(&[[1,2,3,4]]));
        assert_eq!(input.snow, [false,true,false,true,true,false,false,true,false]);
    }

    #[test]
    fn permanent_snow_is_independent_of_movement_category_and_type_failures() {
        for source in [
            "categories=broken terrain={a={perm_snow=yes color={1}}}",
            "categories={bad=broken} terrain={a={type=bad perm_snow=yes color={1}}}",
            "categories={bad={movement_cost=broken}} terrain={a={type=bad perm_snow=yes color={1}}}",
            "terrain={a={type={} perm_snow=yes color={1}}}",
        ] {
            let input=snow_inputs(source, terrain(&[[1,1,1,1]]));
            assert!(input.distance([0,0],[1,0],[0,0],4,1).is_err());
            let map=EngineMap { ids:vec![1;4],width:4,height:1,defined:[1].into_iter().collect(),land:vec![false,true] };
            assert_eq!(input.snow_coverage(&map)[&1],Ok(PermanentSnowCoverage {pixel_count:4,snow_pixels:4}));
        }
    }

    #[test]
    fn snow_samples_narrowed_runs_in_engine_rows_and_palette_zero_outside_bitmap() {
        let input=snow_inputs("terrain={a={perm_snow=yes color={1}}}",terrain(&[[0,0,0,0],[1,1,0,0]]));
        let map=EngineMap { ids:vec![1,1,2,2, 2,2,2,2],width:4,height:2,
            defined:[1,2,3].into_iter().collect(),land:vec![false;4] };
        let values=input.snow_coverage(&map);
        assert_eq!(values[&1],Ok(PermanentSnowCoverage {pixel_count:2,snow_pixels:2}));
        assert_eq!(values[&2],Ok(PermanentSnowCoverage {pixel_count:6,snow_pixels:0}));
        assert_eq!(values[&3],Err(PermanentSnowUnavailable::InactiveProvince));
        // A long run narrows to one pixel, rather than contributing its raster histogram.
        let long=EngineMap {ids:vec![1;65_537],width:65_537,height:1,defined:[1].into_iter().collect(),land:vec![false,true]};
        assert_eq!(input.snow_coverage(&long)[&1],Ok(PermanentSnowCoverage {pixel_count:1,snow_pixels:0}));
        let zero=EngineMap {ids:vec![1;65_536],width:65_536,..long};
        assert_eq!(input.snow_coverage(&zero)[&1],Err(PermanentSnowUnavailable::NativePixelCountFault));
        let input=snow_inputs("terrain={a={perm_snow=yes color={0}}}",terrain(&[[2,2,2,2]]));
        let outside=EngineMap {ids:vec![1;8],width:8,height:1,defined:[1].into_iter().collect(),land:vec![false,true]};
        assert_eq!(input.snow_coverage(&outside)[&1],Ok(PermanentSnowCoverage {pixel_count:8,snow_pixels:4}));
    }

    #[test]
    fn unreadable_terrain_pixel_withdraws_only_its_province() {
        let input=Inputs {terrain:Terrain {width:4,height:1,components:1,pitch:4,bytes:vec![1,1]},
            costs:vec![],snow:vec![false,true],movement_error:None};
        let map=EngineMap {ids:vec![1,1,2,2],width:4,height:1,defined:[1,2].into_iter().collect(),land:vec![false;3]};
        let values=input.snow_coverage(&map);
        assert_eq!(values[&1],Ok(PermanentSnowCoverage {pixel_count:2,snow_pixels:2}));
        assert_eq!(values[&2],Err(PermanentSnowUnavailable::Terrain(Missing::TerrainPixelUnwritten)));
    }

    #[test]
    fn permanent_raw_preserves_signed_width_and_two_ordered_wrapping_products() {
        let coverage=PermanentSnowCoverage {pixel_count:3,snow_pixels:1};
        assert_eq!(coverage.raw_at_observed_max(None),Err(PermanentSnowUnavailable::MaximumUnobtained));
        assert_eq!(coverage.raw_at_observed_max(Some(100_000)),Ok(16_666));
        assert_eq!(coverage.raw_at_observed_max(Some(-100_000)),Ok(-16_666));
        assert_eq!(coverage.raw_at_observed_max(Some(1)),Ok(0));
        assert_eq!(coverage.raw_at_observed_max(Some(i64::MAX)),Ok(0));
        assert_eq!(coverage.raw_at_observed_max(Some(i64::MIN)),Ok(0));
        assert_eq!(PermanentSnowCoverage {pixel_count:2_147_483_647,snow_pixels:2_147_483_647}
            .raw_at_observed_max(Some(100_000)),Ok(7_050));
        assert_eq!(PermanentSnowCoverage {pixel_count:0,snow_pixels:0}.raw_at_observed_max(Some(0)),
            Err(PermanentSnowUnavailable::NonPositivePixelCount));
        assert_eq!(PermanentSnowCoverage {pixel_count:0x8000_0000,snow_pixels:0}.raw_at_observed_max(Some(0)),
            Err(PermanentSnowUnavailable::NonPositivePixelCount));
    }

    #[test]
    fn admitted_snow_uses_selected_files_frozen_namespace_and_shared_invented_province() {
        let root=crate::test_support::temp_test_dir("permanent_snow_admission");
        std::fs::create_dir_all(root.join("map")).unwrap();
        std::fs::create_dir_all(root.join("common/terrain")).unwrap();
        std::fs::write(root.join("map/default.map"),b"terrain=terrain.bmp definitions=custom.csv provinces=custom.bmp").unwrap();
        std::fs::write(root.join("map/custom.csv"),b"0;0;0;0;land\n1;1;0;0;land\n").unwrap();
        let mut bitmap=vec![0u8;66];
        bitmap[..2].copy_from_slice(b"BM");
        bitmap[10..14].copy_from_slice(&54u32.to_le_bytes());
        bitmap[14..18].copy_from_slice(&40u32.to_le_bytes());
        bitmap[18..22].copy_from_slice(&4i32.to_le_bytes());
        bitmap[22..26].copy_from_slice(&1i32.to_le_bytes());
        bitmap[26..28].copy_from_slice(&1u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&24u16.to_le_bytes());
        bitmap[54..].copy_from_slice(&[0,0,1, 0,0,1, 0,0,2, 0,0,2]);
        std::fs::write(root.join("map/custom.bmp"),&bitmap).unwrap();
        bitmap[28..30].copy_from_slice(&8u16.to_le_bytes());
        bitmap[54..58].copy_from_slice(&[1,0,1,1]);
        std::fs::write(root.join("map/terrain.bmp"),&bitmap[..58]).unwrap();
        std::fs::write(root.join("common/terrain/snow.txt"),b"categories=broken terrain={a={perm_snow=yes color={1}}}").unwrap();
        let raw=GameFileResolver::vanilla(&root).unwrap();
        assert!(matches!(ProvincePermanentSnow::load(&raw),Err(PermanentSnowUnavailable::ContentAdmission)));
        let (resolver,_)=crate::test_support::admitted_vanilla_resolver(&raw,&Default::default(),"permanent-snow");
        let snow=ProvincePermanentSnow::load(&resolver).unwrap();
        assert_eq!(snow.province(1),Ok(PermanentSnowCoverage {pixel_count:2,snow_pixels:1}));
        assert_eq!(snow.province(2),Ok(PermanentSnowCoverage {pixel_count:2,snow_pixels:2}));
        assert_eq!(snow.province(0),Err(PermanentSnowUnavailable::UnknownProvince));
        assert_eq!(snow.sources().len(),5);
        assert!(snow.admission_lease().manifest_digest_sha256().is_some());
        std::fs::write(root.join("common/terrain/later.txt"),b"terrain={a={perm_snow=no color={1}}}").unwrap();
        assert_eq!(ProvincePermanentSnow::load(&resolver).unwrap().province(2),snow.province(2));
        drop(snow);drop(resolver);drop(raw);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

}
