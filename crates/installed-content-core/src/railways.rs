//! `map/railways.txt`: the static rail network.
//!
//! The loader reads one token stream through the script lexer, not by line:
//! a level, a province count and that many province ids, each resolved and
//! converted to a signed integer, then the next segment from the token after
//! them. A line whose count exceeds its ids takes the next line's first
//! tokens as its own, and the segments after it are read from there.
//!
//! The file is the *authored* network only. A campaign's railways change as
//! they are built and captured; the replay carries `RailwayConstruction` rows
//! but no table of the network as it stands at an hour, so a viewer drawing
//! this file is drawing 1936's rails at any hour. The caller states that.

use std::io;

use crate::{
    ResolvedLayers,
    fields::invalid,
    fixed5::decimal_i32,
    parser::native::{self, Reader},
};

/// One authored rail segment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Railway {
    /// The level token converted to a signed integer.
    pub level: i32,
    /// The ordered province path the rails run through, at least two.
    pub provinces: Vec<i32>,
}

/// Advances and resolves the token; at the end of the text the advance
/// fails and the token is the last one read.
fn next(reader: &mut Reader) -> (bool, i32) {
    let read = reader.advance();
    reader.current = reader.resolve(reader.current.clone());
    (read, decimal_i32(&reader.current.text))
}

/// Parses `map/railways.txt`: a segment starts at every token the advance
/// reads, and only its level read ends the loop at the end of the text, so
/// a segment cut short repeats the last token. Paths of at least two
/// provinces are added; shorter paths are discarded.
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

/// Reads `map/railways.txt` from the resolved layers; a missing file adds
/// no railways.
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

    /// A segment's count can exceed the ids on its line: the next line's
    /// level is then taken as a province, and reading continues from the
    /// following token. At the end of the text the last token is read again.
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
