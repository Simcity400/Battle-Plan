use content_reader_localisation::localisation::{Piece, pieces};

const OPEN: char = '\u{1}';
const NAME: char = '\u{2}';
const CLOSE: char = '\u{3}';
const INDEX: char = '\u{4}';

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityKind {
    Equipment,
    Module,
    SubUnit,
    Building,
    Tactic,
    Technology,
}

impl EntityKind {
    const ALL: [Self; 6] = [
        Self::Equipment,
        Self::Module,
        Self::SubUnit,
        Self::Building,
        Self::Tactic,
        Self::Technology,
    ];

    fn code(self) -> char {
        match self {
            Self::Equipment => 'e',
            Self::Module => 'm',
            Self::SubUnit => 's',
            Self::Building => 'b',
            Self::Tactic => 't',
            Self::Technology => 'r',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span<'a> {
    Piece(Piece<'a>),
    Open { kind: EntityKind, token: &'a str, index: Option<i32> },
    Close,
}

pub(super) fn mark(kind: EntityKind, token: &str, name: &str) -> String {
    format!("{OPEN}{}{token}{NAME}{name}{CLOSE}", kind.code())
}

pub(super) fn mark_at(kind: EntityKind, token: &str, index: Option<i32>, name: &str) -> String {
    match index {
        Some(index) => format!("{OPEN}{}{token}{INDEX}{index}{NAME}{name}{CLOSE}", kind.code()),
        None => mark(kind, token, name),
    }
}

pub(super) fn unmark(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find([OPEN, CLOSE]) {
        out.push_str(&rest[..at]);
        let marker = &rest[at..];
        rest = match marker.strip_prefix(OPEN) {
            Some(body) => body.find(NAME).map_or("", |end| &body[end + NAME.len_utf8()..]),
            None => &marker[CLOSE.len_utf8()..],
        };
    }
    out.push_str(rest);
    out
}

pub fn spans(text: &str) -> Result<Vec<Span<'_>>, String> {
    let mut out = Vec::new();
    let mut open = false;
    let mut rest = text;
    while let Some(at) = rest.find([OPEN, CLOSE]) {
        out.extend(pieces(&rest[..at]).into_iter().map(Span::Piece));
        let marker = &rest[at..];
        if let Some(body) = marker.strip_prefix(OPEN) {
            if open {
                return Err("an entity link opens inside another".into());
            }
            let end = body.find(NAME).ok_or("an entity link names no token")?;
            let mut header = body[..end].chars();
            let code = header.next().ok_or("an entity link names no kind")?;
            let kind = EntityKind::ALL
                .into_iter()
                .find(|kind| kind.code() == code)
                .ok_or_else(|| format!("an entity link names the unknown kind {code:?}"))?;
            let (token, index) = match header.as_str().split_once(INDEX) {
                Some((token, index)) => (
                    token,
                    Some(index.parse::<i32>().ok().filter(|&index| index > 0).ok_or_else(|| format!("an entity link names the template {index:?}"))?),
                ),
                None => (header.as_str(), None),
            };
            out.push(Span::Open { kind, token, index });
            open = true;
            rest = &body[end + NAME.len_utf8()..];
        } else {
            if !open {
                return Err("an entity link closes without opening".into());
            }
            out.push(Span::Close);
            open = false;
            rest = &marker[CLOSE.len_utf8()..];
        }
    }
    if open {
        return Err("an entity link is not closed".into());
    }
    out.extend(pieces(rest).into_iter().map(Span::Piece));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_names_split_into_links_around_game_pieces() {
        let text = format!(
            "Equipment:\u{11}H{}\u{11}!, {}, {}\n",
            mark(EntityKind::Equipment, "mod_equipment", "Alpha"),
            mark(EntityKind::Tactic, "mod_tactic", "Beta"),
            mark(EntityKind::Technology, "mod_technology", "Gamma")
        );
        assert_eq!(
            spans(&text).unwrap(),
            [
                Span::Piece(Piece::Text("Equipment:")),
                Span::Piece(Piece::Colour('H')),
                Span::Open {
                    kind: EntityKind::Equipment,
                    token: "mod_equipment",
                    index: None,
                },
                Span::Piece(Piece::Text("Alpha")),
                Span::Close,
                Span::Piece(Piece::Colour('!')),
                Span::Piece(Piece::Text(", ")),
                Span::Open {
                    kind: EntityKind::Tactic,
                    token: "mod_tactic",
                    index: None,
                },
                Span::Piece(Piece::Text("Beta")),
                Span::Close,
                Span::Piece(Piece::Text(", ")),
                Span::Open {
                    kind: EntityKind::Technology,
                    token: "mod_technology",
                    index: None,
                },
                Span::Piece(Piece::Text("Gamma")),
                Span::Close,
                Span::Piece(Piece::Text("\n")),
            ]
        );
        assert_eq!(unmark(&text), "Equipment:\u{11}HAlpha\u{11}!, Beta, Gamma\n");
    }

    #[test]
    fn broken_links_are_refused() {
        assert!(spans("\u{1}eunclosed").is_err());
        assert!(spans("\u{1}etoken\u{2}name").is_err());
        assert!(spans("name\u{3}").is_err());
        assert!(spans("\u{1}xtoken\u{2}name\u{3}").is_err());
        assert_eq!(spans("plain").unwrap(), [Span::Piece(Piece::Text("plain"))]);
    }
}
