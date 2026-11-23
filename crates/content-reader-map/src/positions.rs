
use std::io;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitStack {
    pub province: u32,
    pub kind: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
    pub scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuildingPosition {
    pub state: u32,
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
    pub adjacent_sea: u32,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn lines(input: &[u8], what: &str) -> io::Result<Vec<(usize, Vec<String>)>> {
    let input = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input);
    let text = std::str::from_utf8(input).map_err(|_| invalid(format!("{what} is not UTF-8")))?;
    Ok(text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|(index, line)| {
            (
                index + 1,
                line.split(';')
                    .map(|field| field.trim().to_owned())
                    .collect(),
            )
        })
        .collect())
}

fn field<'a>(fields: &'a [String], index: usize, what: &str, line: usize) -> io::Result<&'a str> {
    fields.get(index).map(String::as_str).ok_or_else(|| {
        invalid(format!(
            "{what} line {line} has only {} fields",
            fields.len()
        ))
    })
}

fn integer(
    fields: &[String],
    index: usize,
    what: &str,
    line: usize,
    name: &str,
) -> io::Result<u32> {
    let value = field(fields, index, what, line)?;
    value
        .parse()
        .map_err(|_| invalid(format!("{what} line {line} has a bad {name}: {value:?}")))
}

fn float(fields: &[String], index: usize, what: &str, line: usize, name: &str) -> io::Result<f32> {
    let value = field(fields, index, what, line)?;
    let parsed: f32 = value
        .parse()
        .map_err(|_| invalid(format!("{what} line {line} has a bad {name}: {value:?}")))?;
    if !parsed.is_finite() {
        return Err(invalid(format!(
            "{what} line {line} has a non-finite {name}: {value:?}"
        )));
    }
    Ok(parsed)
}

pub fn parse_unit_stacks(input: &[u8]) -> io::Result<Vec<UnitStack>> {
    let what = "unitstacks.txt";
    let mut stacks = Vec::new();
    for (line, fields) in lines(input, what)? {
        if fields.len() != 7 {
            return Err(invalid(format!(
                "{what} line {line} has {} fields, not 7",
                fields.len()
            )));
        }
        stacks.push(UnitStack {
            province: integer(&fields, 0, what, line, "province")?,
            kind: integer(&fields, 1, what, line, "type")?,
            x: float(&fields, 2, what, line, "x")?,
            y: float(&fields, 3, what, line, "y")?,
            z: float(&fields, 4, what, line, "z")?,
            rotation: float(&fields, 5, what, line, "rotation")?,
            scale: float(&fields, 6, what, line, "scale")?,
        });
    }
    Ok(stacks)
}

fn strtof(field: &[u8]) -> f32 {
    let text = content_reader_parser::parser::c_string(field);
    if let Some(value) = content_reader_parser::crt::scan_f32(text) {
        return value;
    }
    let space = text
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'..=b'\r'))
        .count();
    let start = space + usize::from(matches!(text.get(space), Some(b'+' | b'-')));
    let rest = &text[start..];
    let named = |word: &[u8]| {
        rest.get(..word.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(word))
    };
    let end = if named(b"inf") || named(b"nan") {
        start + 3
    } else {
        let hexadecimal = matches!(rest, [b'0', b'x' | b'X', ..]);
        let radix = if hexadecimal { 16 } else { 10 };
        let digits = |from: usize| {
            rest[from..]
                .iter()
                .take_while(|byte| char::from(**byte).is_digit(radix))
                .count()
        };
        let mut at = if hexadecimal { 2 } else { 0 };
        let whole = digits(at);
        at += whole;
        let mut fraction = 0;
        if rest.get(at) == Some(&b'.') {
            fraction = digits(at + 1);
            at += 1 + fraction;
        }
        match (whole + fraction > 0, hexadecimal) {
            (true, _) => start + at,
            (false, true) => start + 1,
            (false, false) => return 0.0,
        }
    };
    content_reader_parser::crt::scan_f32(&text[..end]).unwrap_or(0.0)
}

fn strtol(text: &[u8]) -> i32 {
    let space = text
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'..=b'\r'))
        .count();
    let sign = usize::from(matches!(text.get(space), Some(b'+' | b'-')));
    let digits = text[space + sign..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    content_reader_parser::fixed5::decimal_i32(&text[..space + sign + digits])
}

pub fn parse_buildings(input: &[u8]) -> io::Result<Vec<BuildingPosition>> {
    let what = "buildings.txt";
    let mut buildings = Vec::new();
    let mut line_start = 0;
    for (index, line) in input.split(|byte| *byte == b'\n').enumerate() {
        let fields: Vec<&[u8]> = line.split(|byte| *byte == b';').collect();
        let from = line_start;
        line_start += line.len() + 1;
        if fields.len() != 7 {
            continue;
        }
        let number = index + 1;
        let integer = |field: usize| {
            let offset: usize = fields[..field].iter().map(|text| text.len() + 1).sum();
            strtol(&input[from + offset..]) as u32
        };
        let float = |field: usize, name: &str| {
            let value = strtof(fields[field]);
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!(
                    "{what} line {number} has a non-finite {name}: {:?}",
                    String::from_utf8_lossy(fields[field])
                )))
            }
        };
        buildings.push(BuildingPosition {
            state: integer(0),
            kind: std::str::from_utf8(fields[1])
                .map_err(|_| {
                    invalid(format!(
                        "{what} line {number} has a building type that is not UTF-8"
                    ))
                })?
                .to_owned(),
            x: float(2, "x")?,
            y: float(3, "y")?,
            z: float(4, "z")?,
            rotation: float(5, "rotation")?,
            adjacent_sea: integer(6),
        });
    }
    Ok(buildings)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherPosition {
    pub line: usize,
    pub region: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub small: bool,
}

pub fn parse_weather_positions(input: &[u8]) -> (Vec<WeatherPosition>, Vec<(usize, usize)>) {
    let mut anchors = Vec::new();
    let mut skipped = Vec::new();
    for (index, line) in input.split(|byte| *byte == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let line = line.split(|byte| *byte == b'\r').next().unwrap_or_default();
        let fields: Vec<&[u8]> = line.split(|byte| *byte == b';').collect();
        let [region, x, y, z, size] = fields[..] else {
            skipped.push((index + 1, fields.len()));
            continue;
        };
        let size = size
            .iter()
            .map(u8::to_ascii_lowercase)
            .skip_while(|byte| matches!(byte, b' ' | b'\t'..=b'\r'))
            .take(5)
            .collect::<Vec<_>>();
        anchors.push(WeatherPosition {
            line: index + 1,
            region: strtol(region),
            x: strtof(x),
            y: strtof(y),
            z: strtof(z),
            small: size == b"small",
        });
    }
    (anchors, skipped)
}

pub fn read_unit_stacks(layers: &content_reader_layers::ResolvedLayers) -> io::Result<Vec<UnitStack>> {
    parse_unit_stacks(&layers.read("map/unitstacks.txt")?)
}

pub fn read_buildings(layers: &content_reader_layers::ResolvedLayers) -> io::Result<Vec<BuildingPosition>> {
    match layers.read("map/buildings.txt") {
        Ok(bytes) => parse_buildings(&bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_stacks_keep_their_decimals_and_negative_rotations() {
        let stacks = parse_unit_stacks(
            b"\xef\xbb\xbf2;0;3429.00;9.85;1699.00;0.00;0.30\n\
              13413;38;3002.80;9.70;1353.55;-1.57;0.20\n\n",
        )
        .unwrap();
        assert_eq!(
            stacks,
            vec![
                UnitStack {
                    province: 2,
                    kind: 0,
                    x: 3429.0,
                    y: 9.85,
                    z: 1699.0,
                    rotation: 0.0,
                    scale: 0.30,
                },
                UnitStack {
                    province: 13413,
                    kind: 38,
                    x: 3002.80,
                    y: 9.70,
                    z: 1353.55,
                    rotation: -1.57,
                    scale: 0.20,
                },
            ]
        );
    }

    #[test]
    fn buildings_keep_their_type_names_and_sea_province() {
        let buildings =
            parse_buildings(b"1;arms_factory;2946.48;11.63;1364.00;0.45;0\n5;naval_base_spawn;1.0;2.0;3.0;0.0;4321\n")
                .unwrap();
        assert_eq!(buildings[0].kind, "arms_factory");
        assert_eq!(buildings[0].x, 2946.48);
        assert_eq!(buildings[0].adjacent_sea, 0);
        assert_eq!(buildings[1].adjacent_sea, 4321);
    }

    #[test]
    fn malformed_lines_name_their_line_number_and_field() {
        let error = parse_unit_stacks(b"2;0;1;2;3;4;5\n3;0;1;2;3;4\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("line 2"), "{error}");
        assert!(error.contains("6 fields, not 7"), "{error}");
        let error = parse_unit_stacks(b"2;0;1;2;3;4;5\n\n4;x;1;2;3;4;5\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("line 3"), "{error}");
        assert!(error.contains("bad type"), "{error}");
        let error = parse_unit_stacks(b"2;0;nan;2;3;4;5\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("non-finite x"), "{error}");
        assert!(parse_unit_stacks(b"\xff\xfe\n").is_err());
        assert!(parse_buildings(b"1;\xff;1;2;3;4;0").is_err());
        assert!(parse_buildings(b"1;a;inf;2;3;4;0").is_err());
    }

    #[test]
    fn buildings_read_as_the_loader_reads_them() {
        let buildings = parse_buildings(
            b"# a;b;c;d;e;f\n\
              5;naval_base_spawn;1.0;2.0;3.0;0.0;4321 # note\r\n\
              1;a;1;2\n\
              \n\
              7;b;1e;;x;0x1p4;\n\
              -3;c;1;2;3;4;-1",
        )
        .unwrap();
        assert_eq!(buildings.len(), 3);
        assert_eq!(buildings[0].adjacent_sea, 4321);
        let b = &buildings[1];
        assert_eq!((b.x, b.y, b.z, b.rotation), (1.0, 0.0, 0.0, 16.0));
        assert_eq!(b.adjacent_sea, -3i32 as u32);
        assert_eq!(
            (buildings[2].state, buildings[2].adjacent_sea),
            (-3i32 as u32, u32::MAX)
        );
    }

    #[test]
    fn coordinates_back_off_to_the_longest_float_prefix_in_one_pass() {
        let every_prefix = |field: &[u8]| {
            (0..=field.len())
                .rev()
                .find_map(|end| content_reader_parser::crt::scan_f32(&field[..end]))
                .unwrap_or(0.0)
        };
        for field in [
            &b""[..],
            b" \t",
            b"-",
            b"+.",
            b"x",
            b"1e",
            b"1e+",
            b".5e",
            b"-2.5E-x",
            b"0x",
            b" -0X",
            b"0x1p",
            b"0x.p1",
            b"0xp1",
            b"0x1.8p1z",
            b"in",
            b"infin",
            b"-INFINIT",
            b"n",
            b"nan(",
            b"nan(a-b)",
            b"NaN(snan",
            b"7\x00e5",
            b"  12px",
        ] {
            assert_eq!(
                strtof(field).to_bits(),
                every_prefix(field).to_bits(),
                "{}",
                field.escape_ascii()
            );
        }
        let mut row = b"1;custom_building;".to_vec();
        row.resize(row.len() + 1_000_000, b' ');
        row.extend_from_slice(b";2;3;4;0");
        let buildings = parse_buildings(&row).unwrap();
        assert_eq!((buildings[0].x, buildings[0].y), (0.0, 2.0));
    }

    #[test]
    fn weather_positions_read_as_the_loader_reads_them() {
        let (anchors, skipped) = parse_weather_positions(
            b"1;10.5;2;3;SMALL\n\
              12abc; 4 ;5x;;  small_x \r\n\
              \r\n\
              3;1;2;3\n\
              \n\
              -4;1;2;3;big\n",
        );
        assert_eq!(
            anchors,
            [
                WeatherPosition { line: 1, region: 1, x: 10.5, y: 2.0, z: 3.0, small: true },
                WeatherPosition { line: 2, region: 12, x: 4.0, y: 5.0, z: 0.0, small: true },
                WeatherPosition { line: 6, region: -4, x: 1.0, y: 2.0, z: 3.0, small: false },
            ]
        );
        assert_eq!(skipped, [(3, 1), (4, 4)]);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped_without_shifting_line_numbers() {
        let stacks = parse_unit_stacks(b"# header\n\n2;0;1;2;3;4;5\n").unwrap();
        assert_eq!(stacks.len(), 1);
        let error = parse_unit_stacks(b"# header\n\n2;0;1;2;3;4\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("line 3"), "{error}");
    }
}
