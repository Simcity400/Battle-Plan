use content_reader_localisation::localisation::Piece;
use dashboard_effects::effect_description::{EntityKind as Marked, Span, spans};
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EntityKind {
    Equipment,
    Module,
    SubUnit,
    Building,
    Tactic,
    Technology,
}

impl From<Marked> for EntityKind {
    fn from(kind: Marked) -> Self {
        match kind {
            Marked::Equipment => Self::Equipment,
            Marked::Module => Self::Module,
            Marked::SubUnit => Self::SubUnit,
            Marked::Building => Self::Building,
            Marked::Tactic => Self::Tactic,
            Marked::Technology => Self::Technology,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GameRun {
    #[doc = " Text in the game's colour code `tone`; null is the default colour."]
    Text { text: String, tone: Option<String> },
    #[doc = " An inline icon the game draws from this sprite."]
    Icon { sprite: String },
    #[doc = " An inline country flag."]
    Flag { tag: String },
    #[doc = " The highlighted name of a thing the text unlocks, which names that thing's content token, and its recorded template where the text resolved one."]
    Link { entity: EntityKind, token: String, database_index: Option<i32>, text: String, tone: Option<String> },
}

#[derive(Default)]
struct Tones(Vec<Option<char>>);

impl Tones {
    fn apply(&mut self, code: char) {
        if code == '!' && self.0.len() > 1 {
            self.0.pop();
        } else {
            self.0.push((code != '!').then_some(code));
        }
    }

    fn current(&self) -> Option<String> {
        self.0.last().copied().flatten().map(String::from)
    }
}

fn push_text(runs: &mut Vec<GameRun>, text: &str, tone: Option<String>) {
    if text.is_empty() {
        return;
    }
    if let Some(GameRun::Text { text: held, tone: last }) = runs.last_mut()
        && *last == tone
    {
        held.push_str(text);
        return;
    }
    runs.push(GameRun::Text { text: text.to_owned(), tone });
}

fn unshowable(piece: Piece<'_>) -> String {
    format!("{}, which plain text cannot show", piece.describe())
}

type OpenLink<'t> = (EntityKind, &'t str, Option<i32>, String, Option<String>);

fn flush(out: &mut Vec<GameRun>, link: &mut OpenLink<'_>) {
    let (entity, token, database_index, text, tone) = link;
    if !text.is_empty() {
        out.push(GameRun::Link { entity: *entity, token: (*token).to_owned(), database_index: *database_index, text: std::mem::take(text), tone: tone.take() });
    }
}

pub fn runs(text: &str) -> Result<Vec<GameRun>, String> {
    let mut out = Vec::new();
    let mut tones = Tones::default();
    let mut link: Option<OpenLink<'_>> = None;
    for span in spans(text)? {
        match span {
            Span::Open { kind, token, index } => link = Some((kind.into(), token, index, String::new(), None)),
            Span::Close => {
                let mut held = link.take().ok_or("an entity link closes without opening")?;
                flush(&mut out, &mut held);
            }
            Span::Piece(Piece::Colour(code)) => tones.apply(code),
            Span::Piece(Piece::Text(piece)) => match &mut link {
                Some((_, _, _, text, tone)) => {
                    if text.is_empty() {
                        *tone = tones.current();
                    }
                    text.push_str(piece);
                }
                None => push_text(&mut out, piece, tones.current()),
            },
            Span::Piece(Piece::Icon(sprite)) => {
                if let Some(held) = &mut link {
                    flush(&mut out, held);
                }
                out.push(GameRun::Icon { sprite: sprite.to_owned() });
            }
            Span::Piece(Piece::Flag(tag)) => {
                if let Some(held) = &mut link {
                    flush(&mut out, held);
                }
                out.push(GameRun::Flag { tag: tag.to_owned() });
            }
            Span::Piece(piece @ (Piece::Control14 | Piece::Control1A(_))) => return Err(unshowable(piece)),
        }
    }
    Ok(out)
}
