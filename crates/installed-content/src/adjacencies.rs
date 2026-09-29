//! Province adjacencies from `map/adjacencies.csv`.
//!
//! Columns are `From;To;Type;Through;start_x;start_y;stop_x;stop_y;rule;Comment`.
//! `-1` means unset in every numeric column. The shipped file ends with a
//! `-1;-1;;-1;...` sentinel row and a commented copy of the header, both of
//! which are skipped.
//!
//! [`parse`] is the strict reader the map passes use. [`native_rows`] reads
//! the file as the engine's loader does, for the province graph; both live
//! here so the file has one home.

use std::io;

use crate::province_geometry::engine_number;

/// A row as `load_authored_adjacencies@140A53290` reads it: numeric prefixes,
/// the kind word, `Through` and the rule name. Raw values; the province graph
/// validates them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeRow {
    pub from: i32,
    /// A row without a `To` field keeps the previous row's.
    pub to: i32,
    /// 1 sea, 2 river, 3 large river, 4 impassable, else 0.
    pub kind: u8,
    /// The raw `Through` column; -1 when unset.
    pub through: i32,
    /// The rule column, `None` when blank.
    pub rule: Option<String>,
}

/// Authored adjacency kinds (`140A53290`).
pub const SEA_CROSSING: u8 = 1;
pub const IMPASSABLE: u8 = 4;

struct CsvCursor<'a> {
    bytes: &'a [u8],
    position: usize,
    /// The last field ended its line.
    line_ended: bool,
}

impl<'a> CsvCursor<'a> {
    /// The next `;`- or newline-ended field; a line's last field keeps no
    /// `\r` of a CRLF ending. `None` at the end or at a 0xFF byte.
    fn field(&mut self) -> Option<&'a [u8]> {
        let start = self.position;
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position += 1;
            if byte == 0xff {
                return None;
            }
            if byte == b';' || byte == b'\n' {
                self.line_ended = byte == b'\n';
                let field = &self.bytes[start..self.position - 1];
                return Some(field.strip_suffix(b"\r").unwrap_or(field));
            }
        }
        None
    }

    /// Past the rest of the row: nothing more when its last field ended it.
    fn next_row(&mut self) {
        if !std::mem::take(&mut self.line_ended) {
            self.skip_line();
        }
    }

    fn skip_line(&mut self) {
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position += 1;
            if byte == b'\n' || byte == 0xff {
                break;
            }
        }
    }
}

/// `From`; a zero `From` field is skipped without its line, and the next
/// field is read as `From` again.
pub(crate) fn native_rows(authored: &[u8]) -> Vec<NativeRow> {
    let mut cursor = CsvCursor {
        bytes: authored,
        position: 0,
        line_ended: false,
    };
    cursor.skip_line();
    let number = |field: &[u8]| engine_number(&String::from_utf8_lossy(field)) as i32;
    let mut rows = Vec::new();
    let mut to = 0_i32;
    while let Some(field) = cursor.field() {
        let from = number(field);
        if from < 0 {
            break;
        }
        if from == 0 {
            continue;
        }
        if let Some(field) = cursor.field() {
            to = number(field);
        }
        let kind = match cursor.field() {
            Some(b"sea") => SEA_CROSSING,
            Some(b"river") => 2,
            Some(b"river_large") => 3,
            Some(b"impassable") => IMPASSABLE,
            _ => 0,
        };
        let through = cursor.field().map_or(0, number);
        for _ in 0..4 {
            cursor.field();
        }
        let rule = cursor
            .field()
            .filter(|name| !name.is_empty())
            .map(|name| String::from_utf8_lossy(name).into_owned());
        rows.push(NativeRow {
            from,
            to,
            kind,
            through,
            rule,
        });
        // A row without the comment column ends at its rule.
        cursor.next_row();
    }
    rows
}

/// What an adjacency represents; the game writes the empty kind for a strait
/// that only an adjacency rule controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdjacencyKind {
    /// A sea connection, `sea` in the file.
    Sea,
    /// A land connection the engine refuses, `impassable` in the file.
    Impassable,
    /// An empty type column, used by the named strait rules.
    RuleOnly,
    /// Any other authored word, kept as written.
    Other(String),
}

/// One row of `map/adjacencies.csv`.
#[derive(Clone, Debug, PartialEq)]
pub struct Adjacency {
    pub from: u32,
    pub to: u32,
    pub kind: AdjacencyKind,
    /// Province the connection passes through, `None` for `-1`.
    pub through: Option<u32>,
    /// Map-pixel start of the drawn link, `None` when any column is `-1`.
    pub start: Option<(i32, i32)>,
    /// Map-pixel end of the drawn link, `None` when any column is `-1`.
    pub stop: Option<(i32, i32)>,
    /// Adjacency rule name, `None` when the column is empty or `-1`.
    pub rule: Option<String>,
    pub comment: String,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn integer(value: &str, line: usize, name: &str) -> io::Result<i32> {
    value.parse().map_err(|_| {
        invalid(format!(
            "adjacencies.csv line {line} has a bad {name}: {value:?}"
        ))
    })
}

fn province(value: &str, line: usize, name: &str) -> io::Result<u32> {
    u32::try_from(integer(value, line, name)?).map_err(|_| {
        invalid(format!(
            "adjacencies.csv line {line} has a negative {name}: {value:?}"
        ))
    })
}

fn optional_province(value: &str, line: usize, name: &str) -> io::Result<Option<u32>> {
    match integer(value, line, name)? {
        -1 => Ok(None),
        other => u32::try_from(other).map(Some).map_err(|_| {
            invalid(format!(
                "adjacencies.csv line {line} has a negative {name}: {value:?}"
            ))
        }),
    }
}

fn point(x: &str, y: &str, line: usize, name: &str) -> io::Result<Option<(i32, i32)>> {
    let x = integer(x, line, name)?;
    let y = integer(y, line, name)?;
    Ok(if x == -1 || y == -1 {
        None
    } else {
        Some((x, y))
    })
}

/// Parses `map/adjacencies.csv`, skipping the header, the sentinel and blanks.
pub fn parse(input: &[u8]) -> io::Result<Vec<Adjacency>> {
    let input = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input);
    let text = std::str::from_utf8(input).map_err(|_| invalid("adjacencies.csv is not UTF-8"))?;
    let mut adjacencies = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fields = raw.split(';').map(str::trim).collect::<Vec<_>>();
        if fields.first() == Some(&"From") {
            continue;
        }
        // The sentinel row is short by the comment column and names no provinces.
        if fields.first() == Some(&"-1") && fields.get(1) == Some(&"-1") {
            continue;
        }
        if fields.len() != 10 {
            return Err(invalid(format!(
                "adjacencies.csv line {line} has {} fields, not 10",
                fields.len()
            )));
        }
        let kind = match fields[2] {
            "sea" => AdjacencyKind::Sea,
            "impassable" => AdjacencyKind::Impassable,
            "" => AdjacencyKind::RuleOnly,
            other => AdjacencyKind::Other(other.to_owned()),
        };
        adjacencies.push(Adjacency {
            from: province(fields[0], line, "From province")?,
            to: province(fields[1], line, "To province")?,
            kind,
            through: optional_province(fields[3], line, "Through province")?,
            start: point(fields[4], fields[5], line, "start position")?,
            stop: point(fields[6], fields[7], line, "stop position")?,
            rule: match fields[8] {
                "" | "-1" => None,
                name => Some(name.to_owned()),
            },
            comment: fields[9].to_owned(),
        });
    }
    Ok(adjacencies)
}

/// Reads the adjacencies file `map/default.map` names.
pub fn read(layers: &crate::ResolvedLayers) -> io::Result<Vec<Adjacency>> {
    parse(&layers.read(&crate::terrain::read_default_map(layers)?.file("adjacencies")?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"\xef\xbb\xbfFrom;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment\n\
        12251;10321;sea;8313;-1;-1;-1;-1;;Andaman\n\
        8857;2284;sea;7617;1560;1009;1556;1016;PANAMA_CANAL;Panama\n\
        5407;5382;;-1;-1;-1;-1;-1;GIBRALTAR_STRAIT;Gibraltar North\n\
        999;998;impassable;-1;-1;-1;-1;-1;-1;\n\
        -1;-1;;-1;-1;-1;-1;-1;-1 \n\
        #Start province ID;End province ID\n\
        \n";

    #[test]
    fn the_header_sentinel_and_comment_rows_are_skipped() {
        let adjacencies = parse(SAMPLE).unwrap();
        assert_eq!(adjacencies.len(), 4);
        assert_eq!(adjacencies[0].from, 12251);
        assert_eq!(adjacencies[0].through, Some(8313));
        assert_eq!(adjacencies[0].start, None);
        assert_eq!(adjacencies[0].rule, None);
        assert_eq!(adjacencies[0].comment, "Andaman");
    }

    #[test]
    fn positions_rules_and_kinds_keep_their_authored_values() {
        let adjacencies = parse(SAMPLE).unwrap();
        assert_eq!(adjacencies[1].kind, AdjacencyKind::Sea);
        assert_eq!(adjacencies[1].start, Some((1560, 1009)));
        assert_eq!(adjacencies[1].stop, Some((1556, 1016)));
        assert_eq!(adjacencies[1].rule.as_deref(), Some("PANAMA_CANAL"));
        // A strait with no type column is controlled by its rule alone.
        assert_eq!(adjacencies[2].kind, AdjacencyKind::RuleOnly);
        assert_eq!(adjacencies[2].rule.as_deref(), Some("GIBRALTAR_STRAIT"));
        assert_eq!(adjacencies[3].kind, AdjacencyKind::Impassable);
        // A `-1` rule column means unset, exactly as the numeric columns do.
        assert_eq!(adjacencies[3].rule, None);
        assert_eq!(adjacencies[3].comment, "");
    }

    #[test]
    fn an_unknown_type_word_is_kept_rather_than_dropped() {
        let rows = parse(b"1;2;canal;-1;-1;-1;-1;-1;;x\n").unwrap();
        assert_eq!(rows[0].kind, AdjacencyKind::Other("canal".into()));
    }

    #[test]
    fn native_rows_keep_kind_through_and_rule_and_stop_at_the_sentinel() {
        let rows = native_rows(
            b"From;To;Type;Through;a;b;c;d;rule;Comment\n\
              1;5;sea;9;-1;-1;-1;-1;STRAIT;x\n\
              2;3;impassable;-1;-1;-1;-1;-1;;y\n\
              -1;-1;;-1;-1;-1;-1;-1;;\n\
              4;6;sea;-1;-1;-1;-1;-1;;after\n",
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            NativeRow {
                from: 1,
                to: 5,
                kind: SEA_CROSSING,
                through: 9,
                rule: Some("STRAIT".into())
            }
        );
        assert_eq!((rows[1].kind, rows[1].rule.as_deref()), (IMPASSABLE, None));
    }

    #[test]
    fn crlf_rows_without_a_comment_column_keep_every_row_and_clean_rule_names() {
        let rows = native_rows(
            b"From;To;Type;Through;a;b;c;d;rule\r\n\
              1;5;sea;9;-1;-1;-1;-1;STRAIT\r\n\
              2;3;;-1;-1;-1;-1;-1;CANAL\r\n\
              -1;-1;;-1;-1;-1;-1;-1;-1\r\n",
        );
        let rules: Vec<_> = rows.iter().map(|row| row.rule.as_deref()).collect();
        assert_eq!(rules, [Some("STRAIT"), Some("CANAL")]);
        assert_eq!((rows[1].from, rows[1].to), (2, 3));
    }

    #[test]
    fn malformed_rows_name_their_line_and_column() {
        for (input, message) in [
            (b"1;2;sea;-1;-1;-1;-1;-1;;\n1;2;sea\n".as_slice(), "line 2"),
            (b"x;2;sea;-1;-1;-1;-1;-1;;\n", "bad From province"),
            (b"1;2;sea;x;-1;-1;-1;-1;;\n", "bad Through province"),
            (b"1;2;sea;-1;x;-1;-1;-1;;\n", "bad start position"),
            (b"-2;2;sea;-1;-1;-1;-1;-1;;\n", "negative From province"),
            (b"1;2;sea;-2;-1;-1;-1;-1;;\n", "negative Through province"),
        ] {
            let error = parse(input).unwrap_err().to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
        assert!(parse(b"\xff\xfe").is_err());
    }
}
