//! `map/railways.txt`: the static rail network.
//!
//! One segment per line, whitespace separated: a level, a province count and
//! that many province ids forming an ordered path (research report R3 section
//! 7.1). The shipped file has 909 segments at levels 1 to 4.
//!
//! The file is the *authored* network only. A campaign's railways change as
//! they are built and captured; the replay carries `RailwayConstruction` rows
//! but no table of the network as it stands at an hour, so a viewer drawing
//! this file is drawing 1936's rails at any hour. The caller states that.

use std::io;

use crate::{ResolvedLayers, fields::invalid};

/// One authored rail segment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Railway {
    /// The rail level, 1 to 5.
    pub level: u32,
    /// The ordered province path the rails run through.
    pub provinces: Vec<u32>,
}

/// Parses `map/railways.txt`.
///
/// Errors are named `InvalidData` with the one-based line number for a line
/// that is not all integers, a count that does not match the ids that follow,
/// and a segment of fewer than two provinces, which could not be a path.
pub fn parse(input: &[u8]) -> io::Result<Vec<Railway>> {
    let text = String::from_utf8_lossy(input);
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index + 1;
        let mut values = Vec::new();
        for field in line.split_whitespace() {
            values.push(field.parse::<u32>().map_err(|_| {
                invalid(format!(
                    "map/railways.txt line {number}: {field:?} is not an integer"
                ))
            })?);
        }
        let (&level, rest) = values
            .split_first()
            .ok_or_else(|| invalid(format!("map/railways.txt line {number} is empty")))?;
        let (&count, provinces) = rest.split_first().ok_or_else(|| {
            invalid(format!(
                "map/railways.txt line {number} has no province count"
            ))
        })?;
        if provinces.len() != count as usize {
            return Err(invalid(format!(
                "map/railways.txt line {number} declares {count} provinces and lists {}",
                provinces.len()
            )));
        }
        if provinces.len() < 2 {
            return Err(invalid(format!(
                "map/railways.txt line {number} is a path of {} province(s)",
                provinces.len()
            )));
        }
        out.push(Railway {
            level,
            provinces: provinces.to_vec(),
        });
    }
    Ok(out)
}

/// Reads `map/railways.txt` from the resolved layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<Railway>> {
    parse(&layers.read("map/railways.txt")?)
        .map_err(|error| invalid(format!("map/railways.txt: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_level_and_path() {
        let rails = parse(b"3 4 6521 11444 3312 6282 \n2 3 6282 349 3340\n").unwrap();
        assert_eq!(rails.len(), 2);
        assert_eq!(rails[0].level, 3);
        assert_eq!(rails[0].provinces, [6521, 11444, 3312, 6282]);
        assert_eq!(rails[1].provinces, [6282, 349, 3340]);
    }

    #[test]
    fn blank_and_comment_lines_are_skipped() {
        assert!(parse(b"\n# a comment\n\n").unwrap().is_empty());
    }

    #[test]
    fn a_count_that_does_not_match_is_refused() {
        assert!(parse(b"1 3 10 20\n").is_err());
    }

    #[test]
    fn a_one_province_path_is_refused() {
        assert!(parse(b"1 1 10\n").is_err());
    }

    #[test]
    fn a_non_integer_field_names_its_line() {
        let error = parse(b"1 2 10 20\n1 2 x 20\n").unwrap_err().to_string();
        assert!(error.contains("line 2"), "{error}");
    }
}
