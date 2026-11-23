
use std::io;

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{fields::invalid, fixed5::decimal_i32};
use content_reader_script::native::{self, Reader};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Railway {
    pub level: i32,
    pub provinces: Vec<i32>,
}

fn next(reader: &mut Reader) -> (bool, i32) {
    let read = reader.advance();
    reader.current = reader.resolve(reader.current.clone());
    (read, decimal_i32(&reader.current.text))
}

pub fn parse(input: &[u8]) -> Vec<Railway> {
    let mut reader = Reader::new(native::lex(input, native::Mode::KeepMark));
    let mut out = Vec::new();
    loop {
        let (read, level) = next(&mut reader);
        if !read {
            break;
        }
        let (_, count) = next(&mut reader);
        let provinces: Vec<i32> = (0..count.max(0)).map(|_| next(&mut reader).1).collect();
        if provinces.len() >= 2 {
            out.push(Railway { level, provinces });
        }
    }
    out
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<Railway>> {
    match layers.read("map/railways.txt") {
        Ok(bytes) => Ok(parse(&bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(invalid(format!("map/railways.txt: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_level_and_path() {
        let rails = parse(b"3 4 6521 11444 3312 6282 \n2 3 6282 349 3340\n");
        assert_eq!(rails.len(), 2);
        assert_eq!(rails[0].level, 3);
        assert_eq!(rails[0].provinces, [6521, 11444, 3312, 6282]);
        assert_eq!(rails[1].provinces, [6282, 349, 3340]);
    }

    #[test]
    fn comments_and_line_breaks_are_white_space_to_the_lexer() {
        assert!(parse(b"\n# a comment\n\n").is_empty());
        assert_eq!(parse(b"2 3\n10 # x\n 20\n30").len(), 1);
    }

    #[test]
    fn a_short_segment_takes_the_next_line_tokens() {
        let rails = parse(b"2 3 10 20\n4 2 5 6\n");
        assert_eq!(
            rails,
            [
                Railway {
                    level: 2,
                    provinces: vec![10, 20, 4],
                },
                Railway {
                    level: 2,
                    provinces: vec![6; 5],
                },
            ]
        );
    }

    #[test]
    fn a_path_of_fewer_than_two_provinces_adds_nothing() {
        assert!(parse(b"1 1 10\n1 0\n1 -3\n").is_empty());
        assert_eq!(parse(b"1 1 10\n1 2 5 6\n").len(), 1);
    }

    #[test]
    fn tokens_convert_to_signed_integers() {
        let rails = parse(b"x 2 7abc 8\n");
        assert_eq!(rails[0].level, 0);
        assert_eq!(rails[0].provinces, [7, 8]);
    }
}
