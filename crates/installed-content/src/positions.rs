//! World positions for unit stacks and buildings.
//!
//! `map/unitstacks.txt` and `map/buildings.txt` are semicolon lists, not
//! Clausewitz text. Coordinates are game world space: x right, y height, z
//! into the map, the same space `pdxmap.shader` places its meshes in.

use std::io;

/// One anchor on the map for a stack of units of one kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitStack {
    pub province: u32,
    /// Engine stack kind; the file writes it as a number.
    pub kind: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
    pub scale: f32,
}

/// One authored building slot inside a state.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildingPosition {
    pub state: u32,
    /// Building type name, such as `arms_factory` or `naval_base_spawn`.
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
    /// `0` where the file declares no adjacent sea province.
    pub adjacent_sea: u32,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Splits a positions file into its non-empty lines with one-based numbers.
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

/// Reads `map/unitstacks.txt`: `province;type;x;y;z;rotation;scale`.
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

/// Reads `map/buildings.txt`: `state;type;x;y;z;rotation;adjacent_sea`.
pub fn parse_buildings(input: &[u8]) -> io::Result<Vec<BuildingPosition>> {
    let what = "buildings.txt";
    let mut buildings = Vec::new();
    for (line, fields) in lines(input, what)? {
        if fields.len() != 7 {
            return Err(invalid(format!(
                "{what} line {line} has {} fields, not 7",
                fields.len()
            )));
        }
        let kind = field(&fields, 1, what, line)?;
        if kind.is_empty() {
            return Err(invalid(format!("{what} line {line} has no building type")));
        }
        buildings.push(BuildingPosition {
            state: integer(&fields, 0, what, line, "state")?,
            kind: kind.to_owned(),
            x: float(&fields, 2, what, line, "x")?,
            y: float(&fields, 3, what, line, "y")?,
            z: float(&fields, 4, what, line, "z")?,
            rotation: float(&fields, 5, what, line, "rotation")?,
            adjacent_sea: integer(&fields, 6, what, line, "adjacent sea province")?,
        });
    }
    Ok(buildings)
}

/// Reads `map/unitstacks.txt` from the resolved layers.
pub fn read_unit_stacks(layers: &crate::ResolvedLayers) -> io::Result<Vec<UnitStack>> {
    parse_unit_stacks(&layers.read("map/unitstacks.txt")?)
}

/// Reads `map/buildings.txt` from the resolved layers.
pub fn read_buildings(layers: &crate::ResolvedLayers) -> io::Result<Vec<BuildingPosition>> {
    parse_buildings(&layers.read("map/buildings.txt")?)
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
        let error = parse_buildings(b"1;;1;2;3;4;0\n").unwrap_err().to_string();
        assert!(error.contains("no building type"), "{error}");
        assert!(parse_buildings(b"1;a;1;2;3;4;-1\n").is_err());
        assert!(parse_unit_stacks(b"\xff\xfe\n").is_err());
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
