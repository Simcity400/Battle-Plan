//! Province geometry and the subject colour rule from verified game content.

use crate::key_names::KeyNames;
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

use crate::{
    ResolvedLayers,
    fields::invalid,
    ideas, modifiers,
    movement::{bitmap::NativeBitmap, records::engine_number},
    parser::{
        self,
        native::{self, Shape},
    },
};

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
    /// One-based index into `map/default.map`'s continent list; zero is none.
    /// `141530D60` narrows the eighth column's `strtol` to the record's u8.
    pub continent: u8,
}

/// The content half of the political colour: country colours themselves are recorded
/// (`CountryAppearance`), already cosmetic-resolved and HSV-modified.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryColors {
    /// Autonomy definition id -> use_overlord_color. Omitted fields default to false.
    pub use_overlord_color: BTreeMap<Vec<u8>, bool>,
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

/// One `definition.csv` record as `map_definition_record_layout::parse@141530D60`
/// builds it and `append_definition@1415307C0` stores it (u16 id at `+0x00`,
/// red, green and blue at `+0x02..+0x04`, kind `+0x05`, coastal `+0x06`,
/// continent `+0x07`, terrain name `+0x08`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DefinitionRow {
    /// One-based line of the file, for messages.
    pub line: usize,
    pub id: u16,
    pub rgb: [u8; 3],
    /// The kind column's bytes, empty when the row has only four columns.
    pub kind: Vec<u8>,
    pub coastal: bool,
    pub terrain: Vec<u8>,
    pub continent: u8,
}

impl DefinitionRow {
    /// The parser compares the kind bytes exactly: `land` (2), `sea` (4) and
    /// `lake` (8); anything else stores kind 1, which `None` stands for.
    pub fn kind(&self) -> Option<ProvinceKind> {
        match self.kind.as_slice() {
            b"land" => Some(ProvinceKind::Land),
            b"sea" => Some(ProvinceKind::Sea),
            b"lake" => Some(ProvinceKind::Lake),
            _ => None,
        }
    }
}

/// Every record of `definition.csv` in file order, as
/// `read_fixed_definitions@1415329E0` and `map_definition_record_layout::parse@141530D60`
/// read it. The file is read as a C string (it ends at its first NUL) and split
/// on line feeds (`split_byte_views@1424D23B0`, which keeps empty pieces); each
/// line ends at its first carriage return and splits on `;` into at most eight
/// columns. A line of fewer than three columns is logged and skipped. Each
/// number is `strtol` (`parse_signed_integer32@14253D550`, [`engine_number`])
/// from its column's first byte in the whole text, so an empty last column
/// reads on through the line break, and is narrowed to the record's field; the
/// kind and coastal columns compare their exact bytes (`true` is coastal).
/// Absent columns take the parser's defaults: kind 1, not coastal, terrain
/// `unknown`, continent 0.
pub(crate) fn definition_rows(input: &[u8]) -> io::Result<Vec<DefinitionRow>> {
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
            // The parser reads the blue column (view 3) past the three it has.
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

/// The province definitions and the colour lookup the province raster is
/// resolved through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definitions {
    /// Every record by id, the zero sentinel included.
    pub provinces: BTreeMap<u32, ProvinceDefinition>,
    /// Each record's id in file order, which is its native record position.
    pub record_ids: Vec<u16>,
    /// `build_id_raster@141532390`'s lookup: each record in file order whose
    /// id is not zero writes its id under its colour, so a later record takes
    /// a colour an earlier one also declares and record zero's is a miss.
    colors: HashMap<u32, u32>,
}

/// Reads `definition.csv` as the game does (`definition_rows`). Records the
/// map does not model are refused by name: a kind other than `land`, `sea` or
/// `lake`, and two records of one id.
pub fn read_definitions(input: &[u8]) -> io::Result<Definitions> {
    let rows = definition_rows(input)?;
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
        let definition = ProvinceDefinition {
            id,
            rgb: row.rgb,
            kind,
            continent: row.continent,
        };
        if provinces.insert(id, definition).is_some() {
            return Err(invalid(format!("duplicate province id {id}")));
        }
        if id != 0 {
            colors.insert(color_key(row.rgb), id);
        }
    }
    if provinces.is_empty() {
        return Err(invalid("definition.csv contains no provinces"));
    }
    Ok(Definitions {
        provinces,
        record_ids: rows.iter().map(|row| row.id).collect(),
        colors,
    })
}

/// Each record's terrain name by id (`+0x08`, column seven, `unknown` when
/// the row has none), from the rows `definition_rows` reads; two records of
/// one id are refused, as [`read_definitions`] refuses them.
pub fn province_terrains(input: &[u8]) -> io::Result<BTreeMap<u32, Vec<u8>>> {
    let mut terrains = BTreeMap::new();
    for row in definition_rows(input)? {
        if terrains.insert(u32::from(row.id), row.terrain).is_some() {
            return Err(invalid(format!("duplicate province id {}", row.id)));
        }
    }
    Ok(terrains)
}

/// The province definitions of [`read_definitions`].
pub fn parse_definitions(input: &[u8]) -> io::Result<BTreeMap<u32, ProvinceDefinition>> {
    Ok(read_definitions(input)?.provinces)
}

/// Resolves the provinces bitmap as `build_id_raster@141532390` does, through
/// the native map bitmap loader (`NativeBitmap`) and the definitions' colour
/// lookup. A colour no record claims is refused: the game invents a land
/// province for it, which the map does not model.
pub fn decode_provinces_bmp(input: &[u8], definitions: &Definitions) -> io::Result<ProvinceImage> {
    let bitmap = NativeBitmap::decode(input)?;
    let count = bitmap
        .width
        .checked_mul(bitmap.height)
        .ok_or_else(|| invalid("BMP pixel count overflows"))?;
    let mut province_ids = Vec::new();
    province_ids
        .try_reserve_exact(count)
        .map_err(|_| invalid("province image exceeds available memory"))?;
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            let rgb = bitmap.rgb(x, bitmap.height - 1 - y)?;
            province_ids.push(*definitions.colors.get(&color_key(rgb)).ok_or_else(|| {
                invalid(format!(
                    "province BMP pixel ({x},{y}) has undefined RGB [{},{},{}]",
                    rgb[0], rgb[1], rgb[2]
                ))
            })?);
        }
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
    #[derive(Clone, Copy)]
    enum Node {
        File,
        State,
        Modifier(modifiers::Node),
        Rule(ideas::RuleNode),
        Mtth,
    }
    let mut undecided = Vec::new();
    // CAutonomousStateDatabase::ReadMember 14066C470 accepts both spellings;
    // vtable 1427CD350 +0x18 is CPersistent::Read, including at file level.
    // CAutonomousState::ReadMember 14066C200 dispatches all fields below.
    // 14066B1B0 constructs each file's lexer in mode 0 (1424A6370).
    let read = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File
                if field.keyword_is("autonomy_state") || field.keyword_is("autonomous_state") =>
            {
                Shape::Object(Node::State)
            }
            Node::File => Shape::Skip,
            Node::State => match field.keyword().as_deref() {
                Some(
                    b"id"
                    | b"default"
                    | b"cost"
                    | b"is_puppet"
                    | b"min_freedom_level"
                    | b"manpower_influence"
                    | b"peace_conference_initial_freedom"
                    | b"use_overlord_color",
                ) => Shape::One,
                Some(
                    b"allowed"
                    | b"can_take_level"
                    | b"can_lose_level"
                    | b"ai_overlord_wants_garrison",
                ) => Shape::Trigger,
                Some(b"modifier") => Shape::Object(Node::Modifier(modifiers::Node::Modifier)),
                Some(b"rule") => Shape::Object(Node::Rule(ideas::RuleNode::Rule)),
                Some(b"allowed_levels_filter") => Shape::List,
                Some(b"ai_subject_wants_higher" | b"ai_overlord_wants_lower") => {
                    Shape::Object(Node::Mtth)
                }
                Some(b"use_for_peace_conference_weight") => Shape::ObjectIfOpen(Node::Mtth),
                _ => Shape::Skip,
            },
            Node::Modifier(node) => {
                modifiers::shape(node, field, modifiers::UNKNOWN, &mut undecided)
                    .map(Node::Modifier)
            }
            Node::Rule(node) => ideas::rule_field(node, field).map(Node::Rule),
            Node::Mtth => native::mean_time_to_happen(field),
        },
    );
    if let Some(cause) = read.unsettled {
        return Err(invalid(format!("autonomy definitions: {cause}")));
    }
    if let Some(cause) = undecided.first() {
        return Err(invalid(format!("autonomy definitions: {cause}")));
    }
    for item in read.items {
        if !item.keyword_is("autonomy_state") && !item.keyword_is("autonomous_state") {
            continue;
        }
        let Some(items) = item.value.items() else {
            continue;
        };
        let mut id = None;
        let mut inherit = false; // 14066C470 initializes +0x2C to zero.
        for field in items {
            if field.keyword_is("id") {
                id = Some(
                    field
                        .value
                        .read_str()
                        .map_err(|error| invalid(format!("autonomy id: {error}")))?,
                );
            } else if field.keyword_is("use_overlord_color") {
                // 1424B06E0 compares C strings, independent of token kind;
                // anything other than exact yes/no leaves the stored byte.
                inherit = field
                    .value
                    .scalar()
                    .and_then(crate::parser::read_bool)
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
    for path in layers
        .paths()
        .filter(|path| path.starts_with("common/autonomous_states/") && path.ends_with(".txt"))
    {
        apply_autonomy_colors(&mut colors, &layers.read(path)?, &names)?;
    }
    Ok(colors)
}

/// Reads the definitions and provinces bitmap `map/default.map` names. The
/// bitmap's size is the engine's `MAP_SIZE` (`CMap::load_province_raster`).
pub fn load(layers: &ResolvedLayers) -> io::Result<MapData> {
    let files = crate::terrain::read_default_map(layers)?;
    let definitions = read_definitions(&layers.read(&files.file("definitions")?)?)?;
    let image = decode_provinces_bmp(&layers.read(&files.file("provinces")?)?, &definitions)?;
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
        let definitions =
            read_definitions(b"0;0;0;0;land\n7;1;2;3;sea\n99;4;5;6;lake\n5;9;9;9;land\n").unwrap();
        let top_row = [3, 2, 1, 6, 5, 4, 222, 222];
        let bottom_row = [9, 9, 9, 3, 2, 1, 111, 111];
        assert_eq!(
            decode_provinces_bmp(&bitmap(2, 2, &[bottom_row, top_row].concat()), &definitions)
                .unwrap(),
            ProvinceImage {
                width: 2,
                height: 2,
                province_ids: vec![7, 99, 5, 7],
            }
        );
    }

    #[test]
    fn the_native_bitmap_loader_reads_what_the_header_says_it_reads() {
        let definitions = read_definitions(b"0;0;0;0;land\n1;4;5;6;land\n").unwrap();
        let good = bitmap(1, 1, &[6, 5, 4, 88]);
        // The loader ignores biSizeImage, and a 24-bit file's pixels start at
        // byte 54 whatever bfOffBits says.
        for (offset, value) in [(34, 999_u32), (10, 40), (2, 1)] {
            let mut bytes = good.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(
                decode_provinces_bmp(&bytes, &definitions)
                    .unwrap()
                    .province_ids,
                [1]
            );
        }
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
        for (offset, value) in [(18, 0_u32), (22, -1_i32 as u32), (30, 1)] {
            let mut bytes = good.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_provinces_bmp(&bytes, &definitions).is_err());
        }
        let mut wrong_bits = good;
        wrong_bits[28..30].copy_from_slice(&32_u16.to_le_bytes());
        assert!(decode_provinces_bmp(&wrong_bits, &definitions).is_err());
    }

    #[test]
    fn a_later_record_takes_a_shared_colour_and_record_zero_never_claims_one() {
        // Millennium Dawn's province 16257 shares record zero's black.
        let definitions =
            read_definitions(b"0;0;0;0;land\n1;0;0;0;land\n2;5;5;5;sea\n3;0;0;0;lake\n").unwrap();
        assert_eq!(definitions.provinces.len(), 4);
        assert_eq!(definitions.record_ids, [0, 1, 2, 3]);
        let image = bitmap(2, 1, &[0, 0, 0, 5, 5, 5, 0, 0]);
        assert_eq!(
            decode_provinces_bmp(&image, &definitions)
                .unwrap()
                .province_ids,
            [3, 2]
        );
        let zero_only = read_definitions(b"0;0;0;0;land\n1;5;5;5;land\n").unwrap();
        assert!(decode_provinces_bmp(&image, &zero_only).is_err());
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
                // A continent's numeric prefix, as strtol reads it.
                (
                    4,
                    1,
                    [1, 2, 221],
                    Some(ProvinceKind::Land),
                    false,
                    b"hills".to_vec(),
                    5
                ),
                // Absent columns take the parser's defaults.
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
                // Narrowed to the record's fields; kinds and `true` are exact bytes.
                (7, 3, [0, 255, 255], None, false, b"plains".to_vec(), 6),
                // An empty last column reads on through the line break.
                (
                    8,
                    5,
                    [9, 9, 9],
                    Some(ProvinceKind::Lake),
                    true,
                    b"lakes".to_vec(),
                    6
                ),
                // The file ends at its first NUL.
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
    }

    #[test]
    fn province_definitions_refuse_forms_the_map_does_not_model() {
        for invalid in [
            "1;1;2;3;land\n1;3;2;1;sea",
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
    fn autonomy_replays_variables_and_keyword_kinds() {
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
}
