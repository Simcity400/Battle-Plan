
use std::io;

use content_reader_parser::{fields::invalid, fixed5::decimal_i32};

pub const SEA_CROSSING: u8 = 1;
pub const RIVER: u8 = 2;
pub const LARGE_RIVER: u8 = 3;
pub const IMPASSABLE: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adjacency {
    pub from: u32,
    pub to: u32,
    pub kind: u8,
    pub through: i32,
    pub start: (i32, i32),
    pub stop: (i32, i32),
    pub rule: Option<Vec<u8>>,
}

struct Csv<'a> {
    bytes: &'a [u8],
    position: usize,
}

struct Field {
    text: Vec<u8>,
    ended: bool,
}

impl Csv<'_> {
    fn next(&mut self) -> Option<u8> {
        let byte = self.bytes.get(self.position).copied();
        self.position += 1;
        byte.filter(|byte| *byte != 0xFF)
    }

    fn field(&mut self) -> Field {
        let mut text = Vec::new();
        loop {
            match self.next() {
                None => {
                    text.push(0);
                    return Field { text, ended: false };
                }
                Some(b';' | b'\n') => return Field { text, ended: true },
                Some(byte) => text.push(byte),
            }
        }
    }

    fn integer(&mut self, value: &mut i32) -> bool {
        let field = self.field();
        if field.ended {
            *value = decimal_i32(&field.text);
        }
        field.ended
    }

    fn skip_line(&mut self) {
        loop {
            match self.next() {
                None | Some(b'\n') => return,
                Some(_) => {}
            }
        }
    }
}

pub fn parse(bytes: &[u8]) -> io::Result<Vec<Adjacency>> {
    let mut csv = Csv { bytes, position: 0 };
    csv.skip_line();
    let (mut to, mut through) = (0, 0);
    let mut rows = Vec::new();
    loop {
        let mut from = 0;
        if !csv.integer(&mut from) || from < 0 {
            break;
        }
        if from == 0 {
            continue;
        }
        csv.integer(&mut to);
        let kind = match csv.field().text.as_slice() {
            b"sea" => SEA_CROSSING,
            b"river" => RIVER,
            b"river_large" => LARGE_RIVER,
            b"impassable" => IMPASSABLE,
            _ => 0,
        };
        csv.integer(&mut through);
        let mut ends = [0; 4];
        for end in &mut ends {
            csv.integer(end);
        }
        let rule = csv.field().text;
        let target = u32::try_from(to).map_err(|_| {
            invalid(format!(
                "adjacency {from};{to} (row {}) names a negative To province, which the game uses unchecked as an index into its province table",
                rows.len() + 1
            ))
        })?;
        rows.push(Adjacency {
            from: from as u32,
            to: target,
            kind,
            through,
            start: (ends[0], ends[1]),
            stop: (ends[2], ends[3]),
            rule: (!rule.is_empty()).then_some(rule),
        });
        csv.skip_line();
    }
    Ok(rows)
}

pub fn read(layers: &content_reader_layers::ResolvedLayers) -> io::Result<Vec<Adjacency>> {
    parse(&layers.read(&content_reader_key_names::terrain::read_default_map(layers)?.file("adjacencies")?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] =
        b"From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment\n\
        12251;10321;sea;8313;-1;-1;-1;-1;;Andaman\n\
        8857;2284;sea;7617;1560;1009;1556;1016;PANAMA_CANAL;Panama\n\
        5407;5382;;-1;-1;-1;-1;-1;GIBRALTAR_STRAIT;Gibraltar North\n\
        999;998;impassable;-1;-1;-1;-1;-1;-1;\n\
        20;21;river_large;-1;-1;-1;-1;-1;;\n\
        -1;-1;;-1;-1;-1;-1;-1;-1 \n\
        4;6;sea;-1;-1;-1;-1;-1;;after\n";

    #[test]
    fn rows_keep_their_stored_values_and_stop_at_the_sentinel() {
        let rows = parse(SAMPLE).unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(
            rows[1],
            Adjacency {
                from: 8857,
                to: 2284,
                kind: SEA_CROSSING,
                through: 7617,
                start: (1560, 1009),
                stop: (1556, 1016),
                rule: Some(b"PANAMA_CANAL".to_vec()),
            }
        );
        assert_eq!((rows[0].start, rows[0].stop), ((-1, -1), (-1, -1)));
        assert_eq!((rows[2].kind, rows[2].through), (0, -1));
        assert_eq!(
            (rows[3].kind, rows[3].rule.as_deref()),
            (IMPASSABLE, Some(&b"-1"[..]))
        );
        assert_eq!(rows[4].kind, LARGE_RIVER);
    }

    #[test]
    fn an_empty_row_is_read_past_field_by_field() {
        let rows =
            parse(b"h\n1;2;sea;9;-1;-1;-1;-1;;a\n;;;;;;;;;\n3;4;sea;9;-1;-1;-1;-1;;b\n").unwrap();
        let pairs: Vec<_> = rows.iter().map(|row| (row.from, row.to)).collect();
        assert_eq!(pairs, [(1, 2), (3, 4)]);
    }

    #[test]
    fn empty_integer_fields_store_zero() {
        let rows = parse(b"h\n5960;5539;sea;;;-1;-1;-1;Strait_of_Malacca;Malacca\n").unwrap();
        assert_eq!((rows[0].through, rows[0].start), (0, (0, -1)));
        assert_eq!(rows[0].rule.as_deref(), Some(&b"Strait_of_Malacca"[..]));
    }

    #[test]
    fn columns_after_the_rule_are_never_read() {
        let rows =
            parse(b"h\n3898;3915;impassable;-1;-1;-1;-1;-1;;Spain-France;Border;;;\n1;2;;-1;-1;-1;-1;-1;;\n")
                .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].kind, rows[0].rule.as_deref()), (IMPASSABLE, None));
    }

    #[test]
    fn a_row_without_a_comment_column_swallows_the_next_line() {
        let rows = parse(
            b"h\r\n1;5;sea;9;-1;-1;-1;-1;STRAIT\r\n2;3;;-1;-1;-1;-1;-1;CANAL\r\n4;6;;-1;-1;-1;-1;-1;;x\r\n",
        )
        .unwrap();
        let read: Vec<_> = rows
            .iter()
            .map(|row| (row.from, row.rule.as_deref()))
            .collect();
        assert_eq!(read, [(1, Some(&b"STRAIT\r"[..])), (4, None)]);
    }

    #[test]
    fn a_row_cut_by_the_end_of_the_file_keeps_what_the_loader_keeps() {
        let rows = parse(b"h\n1;5;sea;9;7;8;7;8;R;x\n2;").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1],
            Adjacency {
                from: 2,
                to: 5,
                kind: 0,
                through: 9,
                start: (0, 0),
                stop: (0, 0),
                rule: Some(b"\0".to_vec()),
            }
        );
        assert_eq!(
            parse(b"h\n1;5;sea;9;-1;-1;-1;-1;;\n\xff3;4").unwrap().len(),
            1
        );
    }

    #[test]
    fn a_negative_to_is_refused() {
        let error = parse(b"h\n1;-5;sea;9;-1;-1;-1;-1;;\n")
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "adjacency 1;-5 (row 1) names a negative To province, which the game uses unchecked as an index into its province table"
        );
    }
}
