use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_script::key_names::KeyNames;
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{fields::invalid, parser};
use crate::movement::{bitmap::NativeBitmap, records::engine_number};

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
    pub continent: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryColors {
    pub use_overlord_color: BTreeMap<Vec<u8>, bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceImage {
    pub width: u32,
    pub height: u32,
    pub province_ids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapData {
    pub width: u32,
    pub height: u32,
    pub province_ids: Vec<u32>,
    pub definitions: BTreeMap<u32, ProvinceDefinition>,
    pub country_colors: CountryColors,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionRow {
    pub line: usize,
    pub id: u16,
    pub rgb: [u8; 3],
    pub kind: Vec<u8>,
    pub coastal: bool,
    pub terrain: Vec<u8>,
    pub continent: u8,
}

impl DefinitionRow {
    pub fn kind(&self) -> Option<ProvinceKind> {
        match self.kind.as_slice() {
            b"land" => Some(ProvinceKind::Land),
            b"sea" => Some(ProvinceKind::Sea),
            b"lake" => Some(ProvinceKind::Lake),
            _ => None,
        }
    }
}

pub fn definition_rows(input: &[u8]) -> io::Result<Vec<DefinitionRow>> {
    let text = parser::c_string(input);
    let mut rows = Vec::new();
    let mut line_start = 0;
    for (index, line) in text.split(|byte| *byte == b'\n').enumerate() {
        let start = line_start;
        line_start += line.len() + 1;
        let row = line.split(|byte| *byte == b'\r').next().unwrap_or_default();
        let mut columns = Vec::with_capacity(8);
        let mut column_start = start;
        for column in row.split(|byte| *byte == b';').take(8) {
            columns.push((column_start, column));
            column_start += column.len() + 1;
        }
        let line = index + 1;
        match columns.len() {
            0..=2 => continue,
            3 => {
                return Err(invalid(format!(
                    "definition.csv line {line} has three columns; the native blue channel is \
                     not determined by the file"
                )));
            }
            _ => {}
        }
        let number = |at: usize| engine_number(&text[columns[at].0..]);
        rows.push(DefinitionRow {
            line,
            id: number(0) as u16,
            rgb: [1, 2, 3].map(|at| number(at) as u8),
            kind: columns.get(4).map_or(Vec::new(), |(_, kind)| kind.to_vec()),
            coastal: columns
                .get(5)
                .is_some_and(|(_, coastal)| *coastal == b"true"),
            terrain: columns
                .get(6)
                .map_or(b"unknown".to_vec(), |(_, terrain)| terrain.to_vec()),
            continent: if columns.len() > 7 {
                number(7) as u8
            } else {
                0
            },
        });
    }
    Ok(rows)
}

fn color_key([red, green, blue]: [u8; 3]) -> u32 {
    (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
}

pub(crate) fn require_positions(rows: &[DefinitionRow]) -> io::Result<()> {
    match rows
        .iter()
        .enumerate()
        .find(|(position, row)| usize::from(row.id) != *position)
    {
        Some((position, row)) => Err(invalid(format!(
            "definition.csv line {} declares province {} at native record {position}; records whose ID differs from their position are unsupported",
            row.line, row.id
        ))),
        None => Ok(()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definitions {
    pub provinces: BTreeMap<u32, ProvinceDefinition>,
    colors: HashMap<u32, u32>,
}

pub fn read_definitions(input: &[u8]) -> io::Result<Definitions> {
    let rows = definition_rows(input)?;
    require_positions(&rows)?;
    let mut provinces = BTreeMap::new();
    let mut colors = HashMap::new();
    for row in &rows {
        let id = u32::from(row.id);
        let kind = row.kind().ok_or_else(|| {
            invalid(format!(
                "definition.csv line {} has province kind {:?}, which the game stores as kind 1; \
                 the map does not support it",
                row.line,
                String::from_utf8_lossy(&row.kind)
            ))
        })?;
        provinces.insert(
            id,
            ProvinceDefinition {
                id,
                rgb: row.rgb,
                kind,
                continent: row.continent,
            },
        );
        if id != 0 {
            colors.insert(color_key(row.rgb), id);
        }
    }
    if provinces.is_empty() {
        return Err(invalid("definition.csv contains no provinces"));
    }
    Ok(Definitions { provinces, colors })
}

pub fn province_terrains(input: &[u8]) -> io::Result<BTreeMap<u32, Vec<u8>>> {
    let rows = definition_rows(input)?;
    require_positions(&rows)?;
    Ok(rows
        .into_iter()
        .map(|row| (u32::from(row.id), row.terrain))
        .collect())
}

pub fn parse_definitions(input: &[u8]) -> io::Result<BTreeMap<u32, ProvinceDefinition>> {
    Ok(read_definitions(input)?.provinces)
}

pub fn decode_provinces_bmp(
    input: &[u8],
    definitions: &mut Definitions,
) -> io::Result<ProvinceImage> {
    let bitmap = NativeBitmap::decode(input)?;
    bitmap.require_rgb()?;
    let count = bitmap
        .width
        .checked_mul(bitmap.height)
        .ok_or_else(|| invalid("BMP pixel count overflows"))?;
    let mut province_ids = Vec::new();
    province_ids
        .try_reserve_exact(count)
        .map_err(|_| invalid("province image exceeds available memory"))?;
    province_ids.resize(count, 0);
    let mut next_id = definitions.provinces.len();
    let mut added_colors = HashMap::new();
    let mut added_provinces = Vec::new();
    for file_row in 0..bitmap.height {
        let y = bitmap.height - 1 - file_row;
        for x in 0..bitmap.width {
            let rgb = bitmap.rgb(x, file_row)?;
            let key = color_key(rgb);
            let id = match definitions
                .colors
                .get(&key)
                .or_else(|| added_colors.get(&key))
                .copied()
            {
                Some(id) => id,
                None => {
                    let id = u16::try_from(next_id).map_err(|_| {
                        invalid(format!(
                            "{} province records exceed the native u16 province IDs",
                            next_id + 1
                        ))
                    })?;
                    let id = u32::from(id);
                    next_id += 1;
                    added_colors.insert(key, id);
                    added_provinces.push(ProvinceDefinition {
                        id,
                        rgb,
                        kind: ProvinceKind::Land,
                        continent: 0,
                    });
                    id
                }
            };
            province_ids[y * bitmap.width + x] = id;
        }
    }
    definitions.colors.extend(added_colors);
    for province in added_provinces {
        definitions.provinces.insert(province.id, province);
    }
    Ok(ProvinceImage {
        width: bitmap.width as u32,
        height: bitmap.height as u32,
        province_ids,
    })
}

fn apply_autonomy_colors(
    colors: &mut CountryColors,
    input: &[u8],
    names: &KeyNames,
) -> io::Result<()> {
    for item in content_reader_key_names::autonomy_definitions::settled_definitions(input, names)? {
        let Some(items) = item.value.items() else {
            continue;
        };
        let mut id = None;
        let mut inherit = false;
        for field in items {
            if field.keyword_is("id") {
                id = Some(
                    field
                        .value
                        .read_str()
                        .map_err(|error| invalid(format!("autonomy id: {error}")))?,
                );
            } else if field.keyword_is("use_overlord_color") {
                inherit = field
                    .value
                    .scalar()
                    .and_then(content_reader_parser::parser::read_bool)
                    .unwrap_or(inherit);
            }
        }
        let id = id.ok_or_else(|| invalid("autonomy_state has no id"))?;
        colors
            .use_overlord_color
            .insert(id.as_bytes().to_vec(), inherit);
    }
    Ok(())
}

pub fn read_country_colors(layers: &ResolvedLayers) -> io::Result<CountryColors> {
    let mut colors = CountryColors::default();
    let names = layers.key_names()?;
    for path in content_reader_layers::script_files::paths(layers, "common/autonomous_states/", Some(".txt"))? {
        apply_autonomy_colors(&mut colors, &path.read(layers)?, &names)?;
    }
    Ok(colors)
}

pub fn load_definitions(layers: &ResolvedLayers) -> io::Result<BTreeMap<u32, ProvinceDefinition>> {
    let files = content_reader_key_names::terrain::read_default_map(layers)?;
    let mut definitions = read_definitions(&layers.read(&files.file("definitions")?)?)?;
    decode_provinces_bmp(&layers.read(&files.file("provinces")?)?, &mut definitions)?;
    Ok(definitions.provinces)
}

pub fn load(layers: &ResolvedLayers) -> io::Result<MapData> {
    let files = content_reader_key_names::terrain::read_default_map(layers)?;
    let mut definitions = read_definitions(&layers.read(&files.file("definitions")?)?)?;
    let image = decode_provinces_bmp(&layers.read(&files.file("provinces")?)?, &mut definitions)?;
    Ok(MapData {
        width: image.width,
        height: image.height,
        province_ids: image.province_ids,
        definitions: definitions.provinces,
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
    fn bmp_bgr_padding_and_file_rows_produce_top_left_ids() {
        let mut definitions =
            read_definitions(b"0;0;0;0;land\n1;1;2;3;sea\n2;4;5;6;lake\n3;9;9;9;land\n").unwrap();
        let top_row = [3, 2, 1, 6, 5, 4, 222, 222];
        let bottom_row = [9, 9, 9, 3, 2, 1, 111, 111];
        assert_eq!(
            decode_provinces_bmp(
                &bitmap(2, 2, &[bottom_row, top_row].concat()),
                &mut definitions,
            )
            .unwrap(),
            ProvinceImage {
                width: 2,
                height: 2,
                province_ids: vec![1, 2, 3, 1],
            }
        );
    }

    #[test]
    fn the_native_bitmap_loader_reads_what_the_header_says_it_reads() {
        let mut definitions = read_definitions(b"0;0;0;0;land\n1;4;5;6;land\n").unwrap();
        let good = bitmap(1, 1, &[6, 5, 4, 88]);
        for (offset, value) in [(34, 999_u32), (10, 40), (2, 1)] {
            let mut bytes = good.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(
                decode_provinces_bmp(&bytes, &mut definitions)
                    .unwrap()
                    .province_ids,
                [1]
            );
        }
        let unknown = bitmap(1, 1, &[3, 2, 1, 88]);
        let mut with_unclaimed = definitions.clone();
        assert_eq!(
            decode_provinces_bmp(&unknown, &mut with_unclaimed)
                .unwrap()
                .province_ids,
            [2]
        );
        assert_eq!(
            with_unclaimed.provinces.get(&2),
            Some(&ProvinceDefinition {
                id: 2,
                rgb: [1, 2, 3],
                kind: ProvinceKind::Land,
                continent: 0,
            })
        );
        for length in 0..good.len() {
            assert!(decode_provinces_bmp(&good[..length], &mut definitions).is_err());
        }
        for (offset, value) in [(18, 0_u32), (22, -1_i32 as u32), (30, 1)] {
            let mut bytes = good.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_provinces_bmp(&bytes, &mut definitions).is_err());
        }
        let mut wrong_bits = good;
        wrong_bits[28..30].copy_from_slice(&32_u16.to_le_bytes());
        assert!(decode_provinces_bmp(&wrong_bits, &mut definitions).is_err());
    }

    #[test]
    fn a_later_record_takes_a_shared_colour_and_record_zero_never_claims_one() {
        let mut definitions =
            read_definitions(b"0;0;0;0;land\n1;0;0;0;land\n2;5;5;5;sea\n3;0;0;0;lake\n").unwrap();
        assert_eq!(definitions.provinces.len(), 4);
        let image = bitmap(2, 1, &[0, 0, 0, 5, 5, 5, 0, 0]);
        assert_eq!(
            decode_provinces_bmp(&image, &mut definitions)
                .unwrap()
                .province_ids,
            [3, 2]
        );
        let mut zero_only = read_definitions(b"0;0;0;0;land\n1;5;5;5;land\n").unwrap();
        assert_eq!(
            decode_provinces_bmp(&image, &mut zero_only)
                .unwrap()
                .province_ids,
            [2, 1]
        );
        assert_eq!(
            zero_only.provinces.get(&2),
            Some(&ProvinceDefinition {
                id: 2,
                rgb: [0, 0, 0],
                kind: ProvinceKind::Land,
                continent: 0,
            })
        );
    }

    #[test]
    fn definition_rows_follow_the_native_parser() {
        let rows = definition_rows(
            b"# a comment\n\
              garbage;row\n\
              \n\
              1;1;2;221 ;land;false;hills;5k\r\n\
              2;3;4;5\n\
              3;6;7;8;sea\n\
              65539;256;-1;99999999999;Land;True;plains;6 ;extra\n\
              5;9;9;9;lake;true;lakes;\r\n\
              6;10;10;10;land;false;plains;7\0\n7;1;1;1;land",
        )
        .unwrap();
        let fields = |row: &DefinitionRow| {
            (
                row.line,
                row.id,
                row.rgb,
                row.kind(),
                row.coastal,
                row.terrain.clone(),
                row.continent,
            )
        };
        let rows: Vec<_> = rows.iter().map(fields).collect();
        assert_eq!(
            rows,
            [
                (
                    4,
                    1,
                    [1, 2, 221],
                    Some(ProvinceKind::Land),
                    false,
                    b"hills".to_vec(),
                    5
                ),
                (5, 2, [3, 4, 5], None, false, b"unknown".to_vec(), 0),
                (
                    6,
                    3,
                    [6, 7, 8],
                    Some(ProvinceKind::Sea),
                    false,
                    b"unknown".to_vec(),
                    0
                ),
                (7, 3, [0, 255, 255], None, false, b"plains".to_vec(), 6),
                (
                    8,
                    5,
                    [9, 9, 9],
                    Some(ProvinceKind::Lake),
                    true,
                    b"lakes".to_vec(),
                    6
                ),
                (
                    9,
                    6,
                    [10, 10, 10],
                    Some(ProvinceKind::Land),
                    false,
                    b"plains".to_vec(),
                    7
                ),
            ]
        );
        assert!(definition_rows(b"1;1;2\n").is_err());
    }

    #[test]
    fn province_terrains_are_the_rows_terrain_columns() {
        let terrains = province_terrains(
            b"0;0;0;0;land\n1;1;1;1;land;false; hills \n2;2;2;2;sea;true;ocean;0",
        )
        .unwrap();
        assert_eq!(terrains[&0], b"unknown");
        assert_eq!(terrains[&1], b" hills ");
        assert_eq!(terrains[&2], b"ocean");
        assert!(province_terrains(b"1;1;1;1;land\n1;2;2;2;land").is_err());
        assert!(province_terrains(b"0;0;0;0;land\n2;1;1;1;sea\n1;2;2;2;land").is_err());
    }

    #[test]
    fn records_whose_id_is_not_their_position_are_refused_by_name() {
        for (input, line, id, position) in [
            (&b"0;0;0;0;land\n2;1;1;1;sea\n1;2;2;2;land\n"[..], 2, 2, 1),
            (b"0;0;0;0;land\n1;1;1;1;land\n3;2;2;2;sea\n", 3, 3, 2),
            (b"1;1;1;1;land\n", 1, 1, 0),
        ] {
            let error = read_definitions(input).unwrap_err().to_string();
            assert_eq!(
                error,
                format!(
                    "definition.csv line {line} declares province {id} at native record {position}; records whose ID differs from their position are unsupported"
                )
            );
            assert_eq!(province_terrains(input).unwrap_err().to_string(), error);
            assert_eq!(parse_definitions(input).unwrap_err().to_string(), error);
        }
    }

    #[test]
    fn province_definitions_refuse_forms_the_map_does_not_model() {
        for invalid in [
            "1;1;2;3;land\n1;3;2;1;sea",
            "0;0;0;0;land\n2;1;2;3;sea\n1;3;2;1;land",
            "1;1;2;3;mountain",
            "1;1;2;3",
            "1;1;2",
            "",
            "# only a comment",
        ] {
            assert!(read_definitions(invalid.as_bytes()).is_err(), "{invalid}");
        }
        let definitions =
            parse_definitions(b"\xef\xbb\xbf# header\n0;0;0;0;land;false;unknown;0\n").unwrap();
        assert_eq!(definitions[&0].kind, ProvinceKind::Land);
    }

    #[test]
    fn the_autonomy_colour_switch_defaults_to_off() {
        content_reader_script::test_game::enter();
        let mut colors = CountryColors::default();
        apply_autonomy_colors(&mut colors, b"autonomy_state={id=puppet} autonomy_state={id=union use_overlord_color=yes} autonomy_state={id=free use_overlord_color=no}", &KeyNames::default()).unwrap();
        assert!(!colors.use_overlord_color[b"puppet".as_slice()]);
        assert!(colors.use_overlord_color[b"union".as_slice()]);
        assert!(!colors.use_overlord_color[b"free".as_slice()]);
        apply_autonomy_colors(
            &mut colors,
            b"autonomy_state={id=x use_overlord_color=yes use_overlord_color=maybe}",
            &KeyNames::default(),
        )
        .unwrap();
        assert!(colors.use_overlord_color[b"x".as_slice()]);
    }

    #[test]
    fn autonomy_colours_follow_the_database_file_roster() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("map_autonomy_roster");
        stack.write(
            &stack.base(),
            "common/autonomous_states/a.txt",
            "autonomy_state = { id = shared use_overlord_color = yes }",
        );
        stack.write(
            &stack.base(),
            "common/autonomous_states/backup/b.txt",
            "autonomy_state = { id = shared use_overlord_color = no } autonomy_state = { id = nested }",
        );
        stack.write(
            &stack.addon(),
            "Common/Autonomous_States/Z.txt",
            "autonomy_state = { id = shared use_overlord_color = no } autonomy_state = { id = upper use_overlord_color = yes }",
        );
        let colors = read_country_colors(&stack.layers(&[])).unwrap();
        assert_eq!(
            colors.use_overlord_color,
            BTreeMap::from([(b"shared".to_vec(), true), (b"upper".to_vec(), true)])
        );
    }

    #[test]
    fn autonomy_replays_variables_and_keyword_kinds() {
        content_reader_script::test_game::enter();
        let mut colors = CountryColors::default();
        apply_autonomy_colors(&mut colors, br#"
            @flag="yes"
            "autonomy_state"={id=ignored}
            autonomous_state={ID=union unknown={@flag=no} USE_OVERLORD_COLOR=@flag "use_overlord_color"=no}
            } autonomy_state={id=after}
        "#, &KeyNames::default()).unwrap();
        assert_eq!(
            colors.use_overlord_color,
            BTreeMap::from([(b"union".to_vec(), true)])
        );
        let error = apply_autonomy_colors(
            &mut colors,
            b"autonomy_state={id=x modifier={unresolved={x=1}}}",
            &KeyNames::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unresolved"));
    }

    #[test]
    fn modded_bitmap_allocates_unclaimed_colours_in_file_row_order_and_carries_definitions() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("map_unclaimed_bmp");
        let base = stack.base();
        let addon = stack.addon();
        stack.write(
            &base,
            "map/default.map",
            "definitions=\"definition.csv\" provinces=\"provinces.bmp\" terrain=\"t.bmp\" rivers=\"r.bmp\" heightmap=\"h.bmp\" tree_definition=\"tr.bmp\"",
        );
        stack.write(
            &base,
            "map/definition.csv",
            "0;0;0;0;land;false;unknown;0\n1;10;20;30;sea;false;ocean;0\n2;40;50;60;lake;false;lakes;2\n",
        );
        let bottom_row = [3, 2, 1, 0, 0, 0, 3, 2, 1, 0, 0, 0];
        let top_row = [9, 8, 7, 30, 20, 10, 9, 8, 7, 0, 0, 0];
        let pixels = [bottom_row, top_row].concat();
        let modded_bmp = bitmap(3, 2, &pixels);
        let path = addon.join("map/provinces.bmp");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, modded_bmp).unwrap();

        let map = load(&stack.layers(&[])).unwrap();
        assert_eq!((map.width, map.height), (3, 2));
        assert_eq!(map.province_ids, [5, 1, 5, 3, 4, 3]);
        assert_eq!(
            map.definitions.get(&3),
            Some(&ProvinceDefinition {
                id: 3,
                rgb: [1, 2, 3],
                kind: ProvinceKind::Land,
                continent: 0,
            })
        );
        assert_eq!(
            map.definitions.get(&4),
            Some(&ProvinceDefinition {
                id: 4,
                rgb: [0, 0, 0],
                kind: ProvinceKind::Land,
                continent: 0,
            })
        );
        assert_eq!(
            map.definitions.get(&5),
            Some(&ProvinceDefinition {
                id: 5,
                rgb: [7, 8, 9],
                kind: ProvinceKind::Land,
                continent: 0,
            })
        );
        assert_eq!(map.definitions.len(), 6);
        let mut full = Definitions {
            provinces: (0..=u32::from(u16::MAX))
                .map(|id| {
                    (
                        id,
                        ProvinceDefinition {
                            id,
                            rgb: [0, 0, 0],
                            kind: ProvinceKind::Land,
                            continent: 0,
                        },
                    )
                })
                .collect(),
            colors: HashMap::new(),
        };
        let overflow = bitmap(1, 1, &[1, 2, 3, 0]);
        let error = decode_provinces_bmp(&overflow, &mut full)
            .unwrap_err()
            .to_string();
        assert!(error.contains("u16 province IDs"), "{error}");
        assert_eq!(full.provinces.len(), 1 << 16);
    }

    #[test]
    fn malformed_packed_bmp_fails_rgb_bounds_before_allocating_province_ids() {
        let mut definitions = read_definitions(b"0;0;0;0;land\n1;1;2;3;land\n").unwrap();
        let mut packed = vec![0_u8; 54];
        packed[..2].copy_from_slice(b"BM");
        packed[2..6].copy_from_slice(&54_u32.to_le_bytes());
        packed[10..14].copy_from_slice(&54_u32.to_le_bytes());
        packed[14..18].copy_from_slice(&40_u32.to_le_bytes());
        packed[18..22].copy_from_slice(&4_i32.to_le_bytes());
        packed[22..26].copy_from_slice(&i32::MAX.to_le_bytes());
        packed[26..28].copy_from_slice(&1_u16.to_le_bytes());
        packed[28..30].copy_from_slice(&1_u16.to_le_bytes());
        let error = decode_provinces_bmp(&packed, &mut definitions)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("pixel reads outside the native bitmap storage"),
            "{error}"
        );
    }
}
