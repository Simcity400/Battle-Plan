use crate::*;
use std::fs;
use std::path::Path;

const MAP_BUILDINGS_SOURCE: &str = "map/buildings.txt";
const MAP_UNITSTACKS_SOURCE: &str = "map/unitstacks.txt";
const MAP_RAILWAYS_SOURCE: &str = "map/railways.txt";
const MAP_SUPPLY_NODES_SOURCE: &str = "map/supply_nodes.txt";
const MAP_WEATHERPOSITIONS_SOURCE: &str = "map/weatherpositions.txt";
const COMMON_BUILDINGS_SOURCE: &str = "common/buildings/00_buildings.txt";

#[derive(Clone, Debug, PartialEq)]
pub struct MapBuildingPlacement {
    pub state_id: u32,
    pub building: String,
    pub pos: [f32; 3],
    /// Vanilla stores map building rotations in radians.
    pub rotation: f32,
    pub adjacent_sea_province: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnitStackPlacement {
    pub province_id: u32,
    /// Raw engine-side stack slot kind. The current vanilla file observed by the
    /// gated test uses values 0..=38; the semantic enum is not shipped here.
    pub kind_raw: u32,
    pub pos: [f32; 3],
    pub rotation: f32,
    pub offset: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwaySegment {
    pub level: u32,
    pub provinces: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupplyNode {
    pub level: u32,
    pub province_id: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeatherPosition {
    pub id: u32,
    pub pos: [f32; 3],
    pub size: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowOnMapBuilding {
    pub name: String,
    pub show_on_map: u32,
    pub show_on_map_meshes: Option<u32>,
}

/// Test-only convenience wrapper; production callers pass a `GameFileResolver`.
pub fn map_building_placements_from_game_dir(game_dir: &Path) -> Result<Vec<MapBuildingPlacement>> {
    let resolver = GameFileResolver::vanilla(game_dir.to_path_buf())?;
    map_building_placements_from_resolver(&resolver)
}

pub fn map_building_placements_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<MapBuildingPlacement>> {
    parse_resolved_game_file(
        resolver,
        MAP_BUILDINGS_SOURCE,
        map_building_placements_from_source,
    )
}

pub fn map_building_placements_from_source(source: &str) -> Result<Vec<MapBuildingPlacement>> {
    parse_semicolon_rows(source, MAP_BUILDINGS_SOURCE, 7, |line, fields| {
        Ok(MapBuildingPlacement {
            state_id: parse_u32_field(fields[0], MAP_BUILDINGS_SOURCE, line, "state_id")?,
            building: fields[1].to_string(),
            pos: [
                parse_f32_field(fields[2], MAP_BUILDINGS_SOURCE, line, "x")?,
                parse_f32_field(fields[3], MAP_BUILDINGS_SOURCE, line, "y")?,
                parse_f32_field(fields[4], MAP_BUILDINGS_SOURCE, line, "z")?,
            ],
            rotation: parse_f32_field(fields[5], MAP_BUILDINGS_SOURCE, line, "rotation")?,
            adjacent_sea_province: parse_u32_field(
                fields[6],
                MAP_BUILDINGS_SOURCE,
                line,
                "adjacent_sea_province",
            )?,
        })
    })
}

/// Test-only convenience wrapper; production callers pass a `GameFileResolver`.
pub fn unit_stack_placements_from_game_dir(game_dir: &Path) -> Result<Vec<UnitStackPlacement>> {
    let resolver = GameFileResolver::vanilla(game_dir.to_path_buf())?;
    unit_stack_placements_from_resolver(&resolver)
}

pub fn unit_stack_placements_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<UnitStackPlacement>> {
    parse_resolved_game_file(
        resolver,
        MAP_UNITSTACKS_SOURCE,
        unit_stack_placements_from_source,
    )
}

pub fn unit_stack_placements_from_source(source: &str) -> Result<Vec<UnitStackPlacement>> {
    parse_semicolon_rows(source, MAP_UNITSTACKS_SOURCE, 7, |line, fields| {
        Ok(UnitStackPlacement {
            province_id: parse_u32_field(fields[0], MAP_UNITSTACKS_SOURCE, line, "province_id")?,
            kind_raw: parse_u32_field(fields[1], MAP_UNITSTACKS_SOURCE, line, "type")?,
            pos: [
                parse_f32_field(fields[2], MAP_UNITSTACKS_SOURCE, line, "x")?,
                parse_f32_field(fields[3], MAP_UNITSTACKS_SOURCE, line, "y")?,
                parse_f32_field(fields[4], MAP_UNITSTACKS_SOURCE, line, "z")?,
            ],
            rotation: parse_f32_field(fields[5], MAP_UNITSTACKS_SOURCE, line, "rotation")?,
            offset: parse_f32_field(fields[6], MAP_UNITSTACKS_SOURCE, line, "offset")?,
        })
    })
}

/// Test-only convenience wrapper; production callers pass a `GameFileResolver`.
pub fn railway_segments_from_game_dir(game_dir: &Path) -> Result<Vec<RailwaySegment>> {
    let resolver = GameFileResolver::vanilla(game_dir.to_path_buf())?;
    railway_segments_from_resolver(&resolver)
}

pub fn railway_segments_from_resolver(resolver: &GameFileResolver) -> Result<Vec<RailwaySegment>> {
    parse_resolved_game_file(resolver, MAP_RAILWAYS_SOURCE, railway_segments_from_source)
}

pub fn railway_segments_from_source(source: &str) -> Result<Vec<RailwaySegment>> {
    let text = strip_comments(source);
    let mut segments = Vec::new();
    for (line_index, raw) in text.lines().enumerate() {
        let line = line_index + 1;
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let fields = raw.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 2 {
            return Err(format!(
                "{MAP_RAILWAYS_SOURCE} line {line}: expected level, count, and province path"
            )
            .into());
        }
        let level = parse_u32_field(fields[0], MAP_RAILWAYS_SOURCE, line, "level")?;
        let count = parse_u32_field(fields[1], MAP_RAILWAYS_SOURCE, line, "count")?;
        let province_count = fields.len() - 2;
        if usize::try_from(count).ok() != Some(province_count) {
            return Err(format!(
                "{MAP_RAILWAYS_SOURCE} line {line}: count field {count} does not match {province_count} following province ids"
            )
            .into());
        }
        let provinces = fields[2..]
            .iter()
            .enumerate()
            .map(|(idx, field)| {
                parse_u32_field(
                    field,
                    MAP_RAILWAYS_SOURCE,
                    line,
                    &format!("province_{}", idx + 1),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        segments.push(RailwaySegment { level, provinces });
    }
    Ok(segments)
}

/// Test-only convenience wrapper; production callers pass a `GameFileResolver`.
pub fn supply_nodes_from_game_dir(game_dir: &Path) -> Result<Vec<SupplyNode>> {
    let resolver = GameFileResolver::vanilla(game_dir.to_path_buf())?;
    supply_nodes_from_resolver(&resolver)
}

pub fn supply_nodes_from_resolver(resolver: &GameFileResolver) -> Result<Vec<SupplyNode>> {
    parse_resolved_game_file(resolver, MAP_SUPPLY_NODES_SOURCE, supply_nodes_from_source)
}

pub fn supply_nodes_from_source(source: &str) -> Result<Vec<SupplyNode>> {
    parse_whitespace_rows(source, MAP_SUPPLY_NODES_SOURCE, 2, |line, fields| {
        Ok(SupplyNode {
            level: parse_u32_field(fields[0], MAP_SUPPLY_NODES_SOURCE, line, "level")?,
            province_id: parse_u32_field(fields[1], MAP_SUPPLY_NODES_SOURCE, line, "province_id")?,
        })
    })
}

/// Test-only convenience wrapper; production callers pass a `GameFileResolver`.
pub fn weather_positions_from_game_dir(game_dir: &Path) -> Result<Vec<WeatherPosition>> {
    let resolver = GameFileResolver::vanilla(game_dir.to_path_buf())?;
    weather_positions_from_resolver(&resolver)
}

pub fn weather_positions_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<WeatherPosition>> {
    parse_resolved_game_file(
        resolver,
        MAP_WEATHERPOSITIONS_SOURCE,
        weather_positions_from_source,
    )
}

pub fn weather_positions_from_source(source: &str) -> Result<Vec<WeatherPosition>> {
    parse_semicolon_rows(source, MAP_WEATHERPOSITIONS_SOURCE, 5, |line, fields| {
        Ok(WeatherPosition {
            id: parse_u32_field(fields[0], MAP_WEATHERPOSITIONS_SOURCE, line, "id")?,
            pos: [
                parse_f32_field(fields[1], MAP_WEATHERPOSITIONS_SOURCE, line, "x")?,
                parse_f32_field(fields[2], MAP_WEATHERPOSITIONS_SOURCE, line, "y")?,
                parse_f32_field(fields[3], MAP_WEATHERPOSITIONS_SOURCE, line, "z")?,
            ],
            size: fields[4].to_string(),
        })
    })
}

/// Test-only convenience wrapper; production callers pass a `GameFileResolver`.
pub fn show_on_map_buildings_from_game_dir(game_dir: &Path) -> Result<Vec<ShowOnMapBuilding>> {
    let resolver = GameFileResolver::vanilla(game_dir.to_path_buf())?;
    show_on_map_buildings_from_resolver(&resolver)
}

pub fn show_on_map_buildings_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<ShowOnMapBuilding>> {
    parse_resolved_game_file(
        resolver,
        COMMON_BUILDINGS_SOURCE,
        show_on_map_buildings_from_source,
    )
}

pub fn show_on_map_buildings_from_source(source: &str) -> Result<Vec<ShowOnMapBuilding>> {
    let text = strip_comments(source);
    let buildings_block = one_named_block(&text, 0, text.len(), "buildings")?
        .ok_or_else(|| format!("{COMMON_BUILDINGS_SOURCE}: missing required buildings block"))?;

    let mut out = Vec::new();
    for building_block in
        top_level_blocks(&text, buildings_block.body_start, buildings_block.body_end)?
    {
        let assignments = top_level_scalar_assignments(
            &text,
            building_block.body_start,
            building_block.body_end,
        )?;
        let mut show_on_map = None;
        let mut show_on_map_meshes = None;

        for assignment in assignments {
            match assignment.key.as_str() {
                "show_on_map" => {
                    if show_on_map.is_some() {
                        return Err(format!(
                            "{COMMON_BUILDINGS_SOURCE} line {}: {} defines show_on_map more than once",
                            assignment.line, building_block.name
                        )
                        .into());
                    }
                    show_on_map = Some(parse_u32_field(
                        &assignment.value,
                        COMMON_BUILDINGS_SOURCE,
                        assignment.line,
                        "show_on_map",
                    )?);
                }
                "show_on_map_meshes" => {
                    if show_on_map_meshes.is_some() {
                        return Err(format!(
                            "{COMMON_BUILDINGS_SOURCE} line {}: {} defines show_on_map_meshes more than once",
                            assignment.line, building_block.name
                        )
                        .into());
                    }
                    show_on_map_meshes = Some(parse_u32_field(
                        &assignment.value,
                        COMMON_BUILDINGS_SOURCE,
                        assignment.line,
                        "show_on_map_meshes",
                    )?);
                }
                _ => {}
            }
        }

        if let Some(show_on_map) = show_on_map {
            out.push(ShowOnMapBuilding {
                name: building_block.name,
                show_on_map,
                show_on_map_meshes,
            });
        }
    }

    Ok(out)
}

fn parse_resolved_game_file<T>(
    resolver: &GameFileResolver,
    source: &str,
    parse: impl FnOnce(&str) -> Result<T>,
) -> Result<T> {
    let path = resolver.resolve_required(source, source)?;
    let bytes = fs::read(&path).map_err(|err| {
        format!(
            "failed to read {source} resolved to {}: {err}",
            path.display()
        )
    })?;
    let text = latin1_bytes_to_string(&bytes);
    parse(&text).map_err(|err| {
        format!(
            "failed to parse {source} resolved to {}: {err}",
            path.display()
        )
        .into()
    })
}

fn parse_semicolon_rows<T>(
    source: &str,
    source_name: &str,
    expected_fields: usize,
    mut parse_row: impl FnMut(usize, &[&str]) -> Result<T>,
) -> Result<Vec<T>> {
    let text = strip_comments(source);
    let mut out = Vec::new();
    for (line_index, raw) in text.lines().enumerate() {
        let line = line_index + 1;
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let fields = raw.split(';').map(str::trim).collect::<Vec<_>>();
        if fields.len() != expected_fields || fields.iter().any(|field| field.is_empty()) {
            return Err(format!(
                "{source_name} line {line}: expected {expected_fields} semicolon fields, got {}",
                fields.len()
            )
            .into());
        }
        out.push(parse_row(line, &fields)?);
    }
    Ok(out)
}

fn parse_whitespace_rows<T>(
    source: &str,
    source_name: &str,
    expected_fields: usize,
    mut parse_row: impl FnMut(usize, &[&str]) -> Result<T>,
) -> Result<Vec<T>> {
    let text = strip_comments(source);
    let mut out = Vec::new();
    for (line_index, raw) in text.lines().enumerate() {
        let line = line_index + 1;
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let fields = raw.split_whitespace().collect::<Vec<_>>();
        if fields.len() != expected_fields {
            return Err(format!(
                "{source_name} line {line}: expected {expected_fields} whitespace fields, got {}",
                fields.len()
            )
            .into());
        }
        out.push(parse_row(line, &fields)?);
    }
    Ok(out)
}

fn parse_u32_field(raw: &str, source: &str, line: usize, field: &str) -> Result<u32> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "{source} line {line}: {field} must be an unsigned integer, got {raw:?}"
        )
        .into());
    }
    raw.parse::<u32>().map_err(|err| {
        format!("{source} line {line}: {field} is not a valid u32 {raw:?}: {err}").into()
    })
}

fn parse_f32_field(raw: &str, source: &str, line: usize, field: &str) -> Result<f32> {
    let value = raw.parse::<f32>().map_err(|err| {
        format!("{source} line {line}: {field} is not a valid f32 {raw:?}: {err}")
    })?;
    if !value.is_finite() {
        return Err(format!("{source} line {line}: {field} must be finite, got {raw:?}").into());
    }
    Ok(value)
}

#[derive(Clone, Debug)]
struct NamedBlock {
    name: String,
    body_start: usize,
    body_end: usize,
}

#[derive(Clone, Debug)]
struct ScalarAssignment {
    key: String,
    value: String,
    line: usize,
}

fn one_named_block(
    text: &str,
    start: usize,
    end: usize,
    wanted: &str,
) -> Result<Option<NamedBlock>> {
    let mut found = None;
    for block in top_level_blocks(text, start, end)? {
        if block.name == wanted {
            if found.is_some() {
                return Err(format!("{COMMON_BUILDINGS_SOURCE}: duplicate {wanted} block").into());
            }
            found = Some(block);
        }
    }
    Ok(found)
}

fn top_level_blocks(text: &str, start: usize, end: usize) -> Result<Vec<NamedBlock>> {
    let bytes = text.as_bytes();
    let mut blocks = Vec::new();
    let mut i = start;
    while i < end {
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end {
            break;
        }
        if !is_word_byte(bytes[i]) {
            i += 1;
            continue;
        }

        let key_start = i;
        while i < end && is_word_byte(bytes[i]) {
            i += 1;
        }
        let key = &text[key_start..i];
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end || bytes[i] != b'{' {
            continue;
        }
        let close = matching_brace(text, i, end)?;
        blocks.push(NamedBlock {
            name: key.to_string(),
            body_start: i + 1,
            body_end: close,
        });
        i = close + 1;
    }
    Ok(blocks)
}

fn top_level_scalar_assignments(
    text: &str,
    start: usize,
    end: usize,
) -> Result<Vec<ScalarAssignment>> {
    let bytes = text.as_bytes();
    let mut assignments = Vec::new();
    let mut i = start;
    while i < end {
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end {
            break;
        }
        if !is_word_byte(bytes[i]) {
            i += 1;
            continue;
        }

        let key_start = i;
        while i < end && is_word_byte(bytes[i]) {
            i += 1;
        }
        let key = &text[key_start..i];
        let line = line_number_at(text, key_start);
        while i < end && bytes[i].is_ascii_whitespace() && bytes[i] != b'\n' {
            i += 1;
        }
        if i >= end || bytes[i] != b'=' {
            if key == "show_on_map" || key == "show_on_map_meshes" {
                return Err(format!(
                    "{COMMON_BUILDINGS_SOURCE} line {line}: {key} must use key = value syntax"
                )
                .into());
            }
            continue;
        }
        i += 1;
        while i < end && matches!(bytes[i], b' ' | b'\t' | b'\r') {
            i += 1;
        }
        if i >= end || bytes[i] == b'\n' || bytes[i] == b'}' {
            return Err(
                format!("{COMMON_BUILDINGS_SOURCE} line {line}: {key} has no value").into(),
            );
        }
        if bytes[i] == b'{' {
            let close = matching_brace(text, i, end)?;
            i = close + 1;
            continue;
        }

        let value_start = i;
        while i < end && !bytes[i].is_ascii_whitespace() && bytes[i] != b'{' && bytes[i] != b'}' {
            i += 1;
        }
        assignments.push(ScalarAssignment {
            key: key.to_string(),
            value: text[value_start..i].to_string(),
            line,
        });
    }
    Ok(assignments)
}

fn matching_brace(text: &str, open: usize, end: usize) -> Result<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    for i in open..end {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => {}
        }
    }
    Err(format!(
        "{COMMON_BUILDINGS_SOURCE} line {}: unclosed block",
        line_number_at(text, open)
    )
    .into())
}

fn line_number_at(text: &str, byte_index: usize) -> usize {
    text.as_bytes()[..byte_index]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::path::{Path, PathBuf};

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    fn path_from_rel(root: &Path, rel: &str) -> PathBuf {
        rel.split('/')
            .filter(|part| !part.is_empty())
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    fn write_text(root: &Path, rel: &str, text: &str) {
        let path = path_from_rel(root, rel);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, text).expect("write fixture file");
    }

    fn write_static_fixture(root: &Path, base: u32) {
        write_text(
            root,
            MAP_BUILDINGS_SOURCE,
            &format!("{base};arms_factory;1.00;2.00;3.00;0.50;0\n"),
        );
        write_text(
            root,
            MAP_UNITSTACKS_SOURCE,
            &format!("{};7;4.00;5.00;6.00;1.50;0.25\n", base + 1),
        );
        write_text(
            root,
            MAP_RAILWAYS_SOURCE,
            &format!("2 3 {} {} {}\n", base + 2, base + 3, base + 4),
        );
        write_text(root, MAP_SUPPLY_NODES_SOURCE, &format!("3 {}\n", base + 5));
        write_text(
            root,
            MAP_WEATHERPOSITIONS_SOURCE,
            &format!("{};7.00;8.00;9.00;large\n", base + 6),
        );
        write_text(
            root,
            COMMON_BUILDINGS_SOURCE,
            &format!(
                "buildings = {{ arms_factory = {{ show_on_map = {} show_on_map_meshes = {} }} }}",
                base + 7,
                base + 8
            ),
        );
    }

    #[test]
    fn parses_building_placements_fixture() {
        let rows = map_building_placements_from_source(
            "1;arms_factory;2946.00;11.63;1364.00;0.45;0\n\
             2;naval_base;123.50;4.00;567.25;-1.25;42\n",
        )
        .expect("parse building placements");

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            MapBuildingPlacement {
                state_id: 1,
                building: "arms_factory".to_string(),
                pos: [2946.0, 11.63, 1364.0],
                rotation: 0.45,
                adjacent_sea_province: 0,
            }
        );
    }

    #[test]
    fn static_data_from_resolver_uses_mod_overrides_for_all_readers() {
        let root = temp_test_dir("static_data_resolver_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_static_fixture(&vanilla, 10);
        write_static_fixture(&mod_root, 100);

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");

        assert_eq!(
            map_building_placements_from_resolver(&resolver)
                .expect("buildings")
                .first()
                .map(|row| row.state_id),
            Some(100)
        );
        assert_eq!(
            unit_stack_placements_from_resolver(&resolver)
                .expect("unit stacks")
                .first()
                .map(|row| row.province_id),
            Some(101)
        );
        assert_eq!(
            railway_segments_from_resolver(&resolver)
                .expect("railways")
                .first()
                .map(|row| row.provinces.clone()),
            Some(vec![102, 103, 104])
        );
        assert_eq!(
            supply_nodes_from_resolver(&resolver)
                .expect("supply")
                .first()
                .map(|row| row.province_id),
            Some(105)
        );
        assert_eq!(
            weather_positions_from_resolver(&resolver)
                .expect("weather")
                .first()
                .map(|row| row.id),
            Some(106)
        );
        assert_eq!(
            show_on_map_buildings_from_resolver(&resolver)
                .expect("show_on_map")
                .first()
                .map(|row| (row.show_on_map, row.show_on_map_meshes)),
            Some((107, Some(108)))
        );
        assert_eq!(resolver.stats().overridden_file_count, 6);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn static_data_vanilla_resolver_matches_direct_source_parse_for_all_readers() {
        let root = temp_test_dir("static_data_resolver_vanilla_equivalence");
        let vanilla = root.join("vanilla");
        write_static_fixture(&vanilla, 30);
        let resolver = GameFileResolver::vanilla(vanilla.clone()).expect("resolver");

        assert_eq!(
            map_building_placements_from_resolver(&resolver).expect("resolver buildings"),
            map_building_placements_from_source(
                &fs::read_to_string(path_from_rel(&vanilla, MAP_BUILDINGS_SOURCE)).unwrap()
            )
            .expect("direct buildings")
        );
        assert_eq!(
            unit_stack_placements_from_resolver(&resolver).expect("resolver unit stacks"),
            unit_stack_placements_from_source(
                &fs::read_to_string(path_from_rel(&vanilla, MAP_UNITSTACKS_SOURCE)).unwrap()
            )
            .expect("direct unit stacks")
        );
        assert_eq!(
            railway_segments_from_resolver(&resolver).expect("resolver railways"),
            railway_segments_from_source(
                &fs::read_to_string(path_from_rel(&vanilla, MAP_RAILWAYS_SOURCE)).unwrap()
            )
            .expect("direct railways")
        );
        assert_eq!(
            supply_nodes_from_resolver(&resolver).expect("resolver supply"),
            supply_nodes_from_source(
                &fs::read_to_string(path_from_rel(&vanilla, MAP_SUPPLY_NODES_SOURCE)).unwrap()
            )
            .expect("direct supply")
        );
        assert_eq!(
            weather_positions_from_resolver(&resolver).expect("resolver weather"),
            weather_positions_from_source(
                &fs::read_to_string(path_from_rel(&vanilla, MAP_WEATHERPOSITIONS_SOURCE)).unwrap()
            )
            .expect("direct weather")
        );
        assert_eq!(
            show_on_map_buildings_from_resolver(&resolver).expect("resolver show_on_map"),
            show_on_map_buildings_from_source(
                &fs::read_to_string(path_from_rel(&vanilla, COMMON_BUILDINGS_SOURCE)).unwrap()
            )
            .expect("direct show_on_map")
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn static_data_from_resolver_honors_replace_path_exclusion() {
        let root = temp_test_dir("static_data_resolver_replace_path");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_static_fixture(&vanilla, 50);
        write_text(
            &mod_root,
            "descriptor.mod",
            "name=\"ReplaceMap\"\nreplace_path=\"map\"\n",
        );

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let err = map_building_placements_from_resolver(&resolver)
            .expect_err("replace_path removes vanilla map file");

        assert!(err.to_string().contains(MAP_BUILDINGS_SOURCE), "{err}");
        assert_eq!(resolver.stats().replace_path_excluded_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn static_data_mod_parse_error_names_mod_file_without_vanilla_fallback() {
        let root = temp_test_dir("static_data_resolver_bad_mod");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_static_fixture(&vanilla, 70);
        write_text(&mod_root, MAP_BUILDINGS_SOURCE, "bad row\n");

        let resolver =
            GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()]).expect("resolver");
        let err = map_building_placements_from_resolver(&resolver).expect_err("bad mod file fails");
        let message = err.to_string();

        assert!(
            message.contains(&mod_root.display().to_string()),
            "{message}"
        );
        assert!(message.contains(MAP_BUILDINGS_SOURCE), "{message}");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn building_placements_fail_closed_on_malformed_rows() {
        let err =
            map_building_placements_from_source("1;arms_factory;2946.00;11.63;1364.00;0.45\n")
                .expect_err("missing field must fail");
        assert!(err.to_string().contains("line 1"));

        let err = map_building_placements_from_source("1.5;arms_factory;1;2;3;0;0\n")
            .expect_err("integer decimal must fail");
        assert!(err.to_string().contains("state_id"));

        let err = map_building_placements_from_source("1;arms_factory;NaN;2;3;0;0\n")
            .expect_err("non-finite float must fail");
        assert!(err.to_string().contains("finite"));
    }

    #[test]
    fn parses_unit_stack_placements_fixture() {
        let rows = unit_stack_placements_from_source("2;0;3429.00;9.85;1699.00;0.00;0.30\n")
            .expect("parse unit stacks");

        assert_eq!(
            rows,
            vec![UnitStackPlacement {
                province_id: 2,
                kind_raw: 0,
                pos: [3429.0, 9.85, 1699.0],
                rotation: 0.0,
                offset: 0.30,
            }]
        );
    }

    #[test]
    fn unit_stack_placements_fail_closed_on_bad_kind() {
        let err = unit_stack_placements_from_source("2;1.5;3429.00;9.85;1699.00;0.00;0.30\n")
            .expect_err("raw kind must remain integer");
        assert!(err.to_string().contains("type"));
    }

    #[test]
    fn parses_railway_segments_fixture_and_count_law() {
        let rows = railway_segments_from_source("3 4 6521 11444 3312 6282\n1 2 10 11\n")
            .expect("parse railways");

        assert_eq!(
            rows,
            vec![
                RailwaySegment {
                    level: 3,
                    provinces: vec![6521, 11444, 3312, 6282],
                },
                RailwaySegment {
                    level: 1,
                    provinces: vec![10, 11],
                },
            ]
        );
    }

    #[test]
    fn railway_segments_fail_closed_on_count_mismatch() {
        let err = railway_segments_from_source("3 4 6521 11444 3312\n")
            .expect_err("count mismatch must fail");
        assert!(err.to_string().contains("count field 4"));
    }

    #[test]
    fn parses_supply_nodes_fixture() {
        let rows = supply_nodes_from_source("1 67\n3 101\n").expect("parse supply nodes");

        assert_eq!(
            rows,
            vec![
                SupplyNode {
                    level: 1,
                    province_id: 67,
                },
                SupplyNode {
                    level: 3,
                    province_id: 101,
                },
            ]
        );
    }

    #[test]
    fn supply_nodes_fail_closed_on_extra_field() {
        let err = supply_nodes_from_source("1 67 99\n").expect_err("extra field must fail");
        assert!(err.to_string().contains("line 1"));
    }

    #[test]
    fn parses_weather_positions_fixture() {
        let rows = weather_positions_from_source("1;2781.24;9.90;1571.49;small\n")
            .expect("parse weather positions");

        assert_eq!(
            rows,
            vec![WeatherPosition {
                id: 1,
                pos: [2781.24, 9.90, 1571.49],
                size: "small".to_string(),
            }]
        );
    }

    #[test]
    fn weather_positions_fail_closed_on_missing_size() {
        let err = weather_positions_from_source("1;2781.24;9.90;1571.49;\n")
            .expect_err("empty size must fail");
        assert!(err.to_string().contains("line 1"));
    }

    #[test]
    fn parses_show_on_map_buildings_fixture() {
        let rows = show_on_map_buildings_from_source(
            r#"
            buildings = {
                arms_factory = {
                    show_on_map = 6
                    base_cost = 7200
                }
                naval_base = {
                    enabled = { always = yes }
                    show_on_map = 1
                    show_on_map_meshes = 3
                }
                hidden_building = {
                    base_cost = 1
                }
            }
            "#,
        )
        .expect("parse show_on_map building rules");

        assert_eq!(
            rows,
            vec![
                ShowOnMapBuilding {
                    name: "arms_factory".to_string(),
                    show_on_map: 6,
                    show_on_map_meshes: None,
                },
                ShowOnMapBuilding {
                    name: "naval_base".to_string(),
                    show_on_map: 1,
                    show_on_map_meshes: Some(3),
                },
            ]
        );
    }

    #[test]
    fn show_on_map_buildings_fail_closed_on_bad_number() {
        let err = show_on_map_buildings_from_source(
            r#"
            buildings = {
                arms_factory = {
                    show_on_map = 6.5
                }
            }
            "#,
        )
        .expect_err("decimal show_on_map must fail");
        assert!(err.to_string().contains("show_on_map"));
        assert!(err.to_string().contains("line"));
    }

    #[test]
    fn gated_vanilla_map_static_data_loads_real_install() {
        let game_dir = Path::new(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla map static data test: {VANILLA_GAME_DIR} is absent");
            return;
        }

        let buildings =
            map_building_placements_from_game_dir(game_dir).expect("parse vanilla buildings.txt");
        let unitstacks =
            unit_stack_placements_from_game_dir(game_dir).expect("parse vanilla unitstacks.txt");
        let railways =
            railway_segments_from_game_dir(game_dir).expect("parse vanilla railways.txt");
        let supply_nodes =
            supply_nodes_from_game_dir(game_dir).expect("parse vanilla supply_nodes.txt");
        let weather =
            weather_positions_from_game_dir(game_dir).expect("parse vanilla weatherpositions.txt");
        let show_on_map =
            show_on_map_buildings_from_game_dir(game_dir).expect("parse vanilla 00_buildings.txt");

        assert_eq!(buildings.len(), 66_664);
        assert_eq!(unitstacks.len(), 265_355);
        assert_eq!(railways.len(), 909);
        assert_eq!(supply_nodes.len(), 727);
        assert_eq!(weather.len(), 588);
        assert_eq!(show_on_map.len(), 29);

        assert_eq!(
            buildings.first(),
            Some(&MapBuildingPlacement {
                state_id: 1,
                building: "arms_factory".to_string(),
                pos: [2946.0, 11.63, 1364.0],
                rotation: 0.45,
                adjacent_sea_province: 0,
            })
        );
        assert_eq!(
            unitstacks.first(),
            Some(&UnitStackPlacement {
                province_id: 2,
                kind_raw: 0,
                pos: [3429.0, 9.85, 1699.0],
                rotation: 0.0,
                offset: 0.30,
            })
        );
        assert_eq!(
            railways.first(),
            Some(&RailwaySegment {
                level: 3,
                provinces: vec![6521, 11444, 3312, 6282],
            })
        );
        assert_eq!(
            supply_nodes.first(),
            Some(&SupplyNode {
                level: 1,
                province_id: 67,
            })
        );
        assert_eq!(
            weather.first(),
            Some(&WeatherPosition {
                id: 1,
                pos: [2781.24, 9.90, 1571.49],
                size: "small".to_string(),
            })
        );
        assert_eq!(
            show_on_map.iter().find(|row| row.name == "arms_factory"),
            Some(&ShowOnMapBuilding {
                name: "arms_factory".to_string(),
                show_on_map: 6,
                show_on_map_meshes: None,
            })
        );
        assert_eq!(
            show_on_map.iter().find(|row| row.name == "naval_base"),
            Some(&ShowOnMapBuilding {
                name: "naval_base".to_string(),
                show_on_map: 1,
                show_on_map_meshes: Some(3),
            })
        );

        let min_kind = unitstacks
            .iter()
            .map(|placement| placement.kind_raw)
            .min()
            .expect("unit stack kind min");
        let max_kind = unitstacks
            .iter()
            .map(|placement| placement.kind_raw)
            .max()
            .expect("unit stack kind max");
        assert_eq!((min_kind, max_kind), (0, 38));

        let min_rotation = buildings
            .iter()
            .map(|placement| placement.rotation)
            .fold(f32::INFINITY, f32::min);
        let max_rotation = buildings
            .iter()
            .map(|placement| placement.rotation)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((-6.05..=-6.03).contains(&min_rotation), "{min_rotation}");
        assert!((6.27..=6.29).contains(&max_rotation), "{max_rotation}");

        // buildings.txt position stats: every locator must be finite and sit
        // in the map's world quadrant (x,z >= 0). Vanilla map world extent is
        // 5632 x 2048; a generous ceiling catches parse-shift corruption
        // without hard-coding an over-tight bound.
        let mut bounds = [[f32::INFINITY, f32::NEG_INFINITY]; 3];
        for placement in &buildings {
            for axis in 0..3 {
                let v = placement.pos[axis];
                assert!(v.is_finite(), "non-finite building coordinate {v}");
                bounds[axis][0] = bounds[axis][0].min(v);
                bounds[axis][1] = bounds[axis][1].max(v);
            }
        }
        assert!(
            bounds[0][0] >= 0.0 && bounds[0][1] <= 5632.0,
            "x {:?}",
            bounds[0]
        );
        assert!(
            bounds[2][0] >= 0.0 && bounds[2][1] <= 2048.0,
            "z {:?}",
            bounds[2]
        );
        assert!(
            bounds[1][0] >= 0.0 && bounds[1][1] <= 200.0,
            "y {:?}",
            bounds[1]
        );
        println!(
            "buildings.txt position stats: {} rows; x[{:.1},{:.1}] y[{:.2},{:.2}] z[{:.1},{:.1}] \
             rotation[{:.3},{:.3}]",
            buildings.len(),
            bounds[0][0],
            bounds[0][1],
            bounds[1][0],
            bounds[1][1],
            bounds[2][0],
            bounds[2][1],
            min_rotation,
            max_rotation
        );

        assert!(railways.iter().all(|segment| !segment.provinces.is_empty()));
        assert!(supply_nodes.iter().all(|node| node.province_id > 0));
    }
}
