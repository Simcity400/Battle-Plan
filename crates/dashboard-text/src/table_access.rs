use crate::invalid;
use replay_data::{
    Hour, rows::LexicalTokenRow, rows::TableRows, rows::TableSchema, rows::tables,
    scope::CaptureMode,
};
#[cfg(test)]
use replay_frame::frame::DecodedFrame;
use replay_frame::frame::TableView;
use std::{collections::BTreeMap, io};

/// What one table of one decoded hour actually holds.
///
/// `Truncated` keeps the rows the collector's partial walk did reach: they are
/// real recorded rows, not a guess, and a pass may draw them. A partial walk
/// never asserts that the absent keys were deleted, so `missing` counts rows
/// the hour is silent about, not rows that are gone.
pub enum Available<'a, T: TableSchema> {
    /// Every row the hour recorded.
    Complete(&'a [T::Row]),
    /// The rows the walk reached, and how many rows it did not reach.
    Truncated { rows: &'a [T::Row], missing: u32 },
    /// The table failed for this hour, with the collector's own reason.
    Unavailable { reason: &'a str },
    /// The recording's capture mode leaves the table out.
    NotRecorded { mode: CaptureMode },
}

impl<'a, T: TableSchema> Available<'a, T> {
    /// The rows the hour holds. An unavailable or unrecorded table holds none.
    pub fn rows(&self) -> &'a [T::Row] {
        match self {
            Available::Complete(rows) | Available::Truncated { rows, .. } => rows,
            Available::Unavailable { .. } | Available::NotRecorded { .. } => &[],
        }
    }

    /// Whether the hour holds the whole table.
    pub fn is_complete(&self) -> bool {
        matches!(self, Available::Complete(_))
    }
}

/// A recorded country reference (a CountryLinks id) as the canonical slot
/// it links to: `None` for 0, no country; `Err` names why this hour cannot
/// say (a link the complete table lacks, or one a truncated or unavailable
/// table may hold). A present link proves either answer of a comparison
/// with it; an unresolved one proves neither.
pub fn country_slot(
    links: &Available<'_, tables::CountryLinks>,
    country: u32,
) -> Result<Option<u32>, String> {
    if country == 0 {
        return Ok(None);
    }
    if let Some(link) = links.rows().iter().find(|link| link.id == country) {
        return Ok(Some(link.canonical_slot));
    }
    Err(match links {
        Available::Complete(_) => format!("country {country} has no CountryLinks row"),
        Available::Truncated { missing, .. } => format!(
            "the CountryLinks table is truncated this hour ({missing} rows not reached) and holds no row for country {country}"
        ),
        Available::Unavailable { reason } => format!(
            "the CountryLinks table is unavailable this hour ({reason}), so country {country} is unresolved"
        ),
        Available::NotRecorded { mode } => format!(
            "the CountryLinks table is not recorded in a {mode} recording, so country {country} is unresolved"
        ),
    })
}

/// The rows of `T` this hour holds, whatever state its table is in.
///
/// The only error is a decoded table holding another table's rows, which is a
/// corrupt file rather than a recorded gap.
pub fn rows<T: TableSchema>(
    frame: &dyn replay_frame::frame::DecodedSource,
) -> io::Result<Available<'_, T>> {
    match frame.view(T::ID.index()) {
        TableView::Complete(rows) => Ok(Available::Complete(typed::<T>(rows, frame.hour())?)),
        TableView::Truncated { rows, missing } => Ok(Available::Truncated {
            rows: typed::<T>(rows, frame.hour())?,
            missing: *missing,
        }),
        TableView::Unavailable { reason } => Ok(Available::Unavailable { reason }),
        TableView::NotRecorded { mode } => Ok(Available::NotRecorded { mode: *mode }),
    }
}

/// Content names spelled by this hour's `LexicalTokens`. Rows record an
/// equipment type, module or upgrade by its token: the engine builds each
/// definition's name and token from one authored key, so the spelling is the
/// name. The exception is token 0, returned for every rejected key
/// (empty, or starting with a digit or
/// '-'), so it names no one key. A spelling that is not NUL-free UTF-8 names
/// nothing here.
#[derive(Clone, Copy, Debug)]
pub struct TokenNames<'a> {
    /// The hour's rows, in token order.
    rows: replay_frame::frame::Rows<'a, LexicalTokenRow>,
    complete: bool,
}

/// Why a token has no name this hour. Formatted only when reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenError {
    Rejected,
    Absent(u32),
    Missing(u32),
    NotText(u32),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected => write!(f, "token 0 stands for every key the lexer rejects"),
            Self::Absent(token) => write!(f, "token {token} is not in LexicalTokens"),
            Self::Missing(token) => {
                write!(f, "token {token} is among LexicalTokens' missing rows")
            }
            Self::NotText(token) => write!(f, "token {token} is not spelled as text"),
        }
    }
}

impl<'a> TokenNames<'a> {
    pub fn new(frame: &'a dyn replay_frame::frame::RecordedSource) -> io::Result<Self> {
        if matches!(
            frame.lexical_status(),
            replay_frame::frame::OperandStatus::WrongLayout
                | replay_frame::frame::OperandStatus::TruncatedWrongLayout(_)
        ) {
            return Err(invalid(format!(
                "hour {} LexicalTokens contains another table's rows",
                frame.hour()
            )));
        }
        let rows = frame.lexical_rows();
        if !rows.is_sorted_by(|a, b| a.token < b.token) {
            return Err(invalid(format!(
                "hour {} LexicalTokens rows are not in token order",
                frame.hour()
            )));
        }
        Ok(Self {
            rows,
            complete: frame.lexical_status().is_complete(),
        })
    }

    /// Rows already in ascending token order, as every replay table is.
    pub fn from_rows(rows: &'a [LexicalTokenRow], complete: bool) -> Self {
        Self {
            rows: rows.into(),
            complete,
        }
    }

    /// Every token of the hour spelled as text, in token order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &'a str)> + '_ {
        self.rows.iter().filter_map(|row| {
            let name = std::str::from_utf8(&row.literal).ok()?;
            (row.token != 0 && !name.contains('\0')).then_some((row.token, name))
        })
    }

    /// The token the lexer gives each key: the one spelled equal to it under
    /// the lexer's fold (`TokenKeys`), found in one pass without allocating.
    /// Several such tokens are refused by name.
    pub fn tokens_of<const N: usize>(&self, keys: [&str; N]) -> [Result<Option<u32>, String>; N] {
        #[derive(Clone, Copy)]
        enum Found {
            None,
            One(u32),
            Two(u32, u32),
        }
        let mut found = [Found::None; N];
        for (token, name) in self.iter() {
            for (key, found) in keys.iter().zip(&mut found) {
                if name.eq_ignore_ascii_case(key) {
                    *found = match *found {
                        Found::None => Found::One(token),
                        Found::One(first) => Found::Two(first, token),
                        two => two,
                    };
                }
            }
        }
        std::array::from_fn(|index| match found[index] {
            Found::None => Ok(None),
            Found::One(token) => Ok(Some(token)),
            Found::Two(first, second) => Err(format!(
                "{} folds to several tokens: {first}, {second} and perhaps more",
                keys[index]
            )),
        })
    }

    /// The token's name, or why this hour cannot give one.
    pub fn name(&self, token: u32) -> Result<&'a str, TokenError> {
        if token == 0 {
            return Err(TokenError::Rejected);
        }
        let Ok(index) = self.rows.binary_search_by_key(&token, |row| row.token) else {
            return Err(if self.complete {
                TokenError::Absent(token)
            } else {
                TokenError::Missing(token)
            });
        };
        std::str::from_utf8(&self.rows.get(index).unwrap().literal)
            .ok()
            .filter(|name| !name.contains('\0'))
            .ok_or(TokenError::NotText(token))
    }
}

/// Refused token matches by reason, with how often each was met.
pub type Refusals = BTreeMap<String, usize>;

pub fn refuse(refusals: &mut Refusals, reason: String) {
    *refusals.entry(reason).or_default() += 1;
}

pub use crate::triggers::fold_token;

/// Content keys grouped by lexer token identity. Lookup uses ASCII-folding
/// FNV-1a, equal length and comparison folding only A-Z. Equivalent keys share
/// one token, so several such content keys make the spelling ambiguous.
#[derive(Clone, Debug, Default)]
pub struct TokenKeys(BTreeMap<String, Vec<String>>);

impl TokenKeys {
    pub fn new<'k>(keys: impl IntoIterator<Item = &'k String>) -> Self {
        let mut folded = BTreeMap::<String, Vec<String>>::new();
        for key in keys {
            folded.entry(fold_token(key)).or_default().push(key.clone());
        }
        Self(folded)
    }

    /// The content key a spelling names; `None` when content has none.
    pub fn resolve(&self, spelling: &str) -> Result<Option<&str>, String> {
        match self.keys(spelling) {
            [] => Ok(None),
            [key] => Ok(Some(key)),
            keys => Err(format!(
                "token {spelling} names several content keys: {}",
                keys.join(", ")
            )),
        }
    }

    /// Each set of content keys equal under the lexer's fold, which every
    /// spelling of them names at once: [`Self::resolve`] refuses them.
    pub fn ambiguous(&self) -> impl Iterator<Item = &[String]> {
        self.0
            .values()
            .map(Vec::as_slice)
            .filter(|keys| keys.len() > 1)
    }

    /// Every content key equal to a spelling under the lexer's fold, in the order given.
    pub fn keys(&self, spelling: &str) -> &[String] {
        self.0.get(&fold_token(spelling)).map_or(&[], Vec::as_slice)
    }
}

fn typed<T: TableSchema>(rows: &TableRows, hour: Hour) -> io::Result<&[T::Row]> {
    rows.rows::<T>().ok_or_else(|| {
        invalid(format!(
            "hour {hour} {:?} contains another table's rows",
            T::ID
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::LexicalTokenRow, rows::TableId};

    #[test]
    fn truncated_lexical_wrong_layout_is_corruption_for_partial_readers() {
        let frame = frame(TableView::Truncated {
            rows: TableRows::Countries(Vec::new()),
            missing: 3,
        });
        assert_eq!(
            TokenNames::new(&frame).unwrap_err().to_string(),
            "hour 0 LexicalTokens contains another table's rows"
        );
        assert_eq!(
            (&frame as &dyn replay_frame::frame::RecordedSource)
                .lexical()
                .unwrap_err()
                .to_string(),
            frame
                .complete::<tables::LexicalTokens>()
                .unwrap_err()
                .to_string()
        );
    }

    fn frame(view: TableView) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Default::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::LexicalTokens.index()] = view;
        frame
    }

    #[test]
    fn token_names_spell_text_and_refuse_everything_else() {
        let rows = TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 7,
                literal: b"light_tank_chassis".to_vec(),
            },
            LexicalTokenRow {
                token: 8,
                literal: b"a\0b".to_vec(),
            },
            LexicalTokenRow {
                token: 9,
                literal: vec![0xff],
            },
        ]);
        let complete = frame(TableView::Complete(rows.clone()));
        let names = TokenNames::new(&complete).unwrap();
        assert_eq!(names.name(7), Ok("light_tank_chassis"));
        for token in [8, 9] {
            assert_eq!(names.name(token), Err(TokenError::NotText(token)));
        }
        assert_eq!(names.name(10), Err(TokenError::Absent(10)));
        // Token 0 is shared by every key the lexer rejects.
        assert_eq!(names.name(0), Err(TokenError::Rejected));
        let truncated = frame(TableView::Truncated { rows, missing: 3 });
        let names = TokenNames::new(&truncated).unwrap();
        assert_eq!(names.name(7), Ok("light_tank_chassis"));
        assert_eq!(names.name(10), Err(TokenError::Missing(10)));
        let unavailable = frame(TableView::Unavailable {
            reason: "not captured".into(),
        });
        assert_eq!(
            TokenNames::new(&unavailable).unwrap().name(7),
            Err(TokenError::Missing(7))
        );
        assert_eq!(
            TokenError::Missing(7).to_string(),
            "token 7 is among LexicalTokens' missing rows"
        );
    }

    #[test]
    fn keys_find_their_one_token_under_the_lexers_fold() {
        let row = |token, literal: &[u8]| LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        };
        let rows = TableRows::LexicalTokens(vec![
            row(0, b"air_base"),
            row(3, b"Air_Base"),
            row(4, b"naval_base"),
            row(5, b"NAVAL_BASE"),
        ]);
        let complete = frame(TableView::Complete(rows));
        let names = TokenNames::new(&complete).unwrap();
        // Token 0 names no one key, so only token 3 spells air_base.
        let [air, bunker, naval] = names.tokens_of(["air_base", "BUNKER", "naval_base"]);
        assert_eq!((air, bunker), (Ok(Some(3)), Ok(None)));
        assert_eq!(
            naval,
            Err("naval_base folds to several tokens: 4, 5 and perhaps more".to_owned())
        );
        assert_eq!(
            names.iter().map(|(token, _)| token).collect::<Vec<_>>(),
            [3, 4, 5]
        );
    }

    #[test]
    fn token_keys_follow_the_lexers_ascii_fold() {
        let keys = [
            "Light_Tank".to_owned(),
            "infantry".to_owned(),
            "INFANTRY".to_owned(),
            "\u{e9}t\u{e9}".to_owned(),
        ];
        let keys = TokenKeys::new(&keys);
        assert_eq!(keys.resolve("light_tank"), Ok(Some("Light_Tank")));
        assert_eq!(keys.resolve("LIGHT_TANK"), Ok(Some("Light_Tank")));
        assert_eq!(keys.resolve("light_tanks"), Ok(None));
        assert_eq!(
            keys.resolve("Infantry"),
            Err("token Infantry names several content keys: infantry, INFANTRY".to_owned())
        );
        // Only A-Z fold: other letters must match exactly.
        assert_eq!(keys.resolve("\u{e9}t\u{e9}"), Ok(Some("\u{e9}t\u{e9}")));
        assert_eq!(keys.resolve("\u{c9}t\u{c9}"), Ok(None));
    }

    /// A present link proves the slot; 0 is no country; a link the hour
    /// lacks is named with why (a complete table without it, a truncated
    /// or unavailable one), never taken as another country's.
    #[test]
    fn country_references_resolve_or_name_why_not() {
        use replay_data::rows::CountryLinkRow;
        let with = |view: TableView| {
            let mut frame = frame(TableView::Complete(TableRows::empty(
                TableId::LexicalTokens,
            )));
            *frame.tables[TableId::CountryLinks.index()] = view;
            frame
        };
        let links = TableRows::CountryLinks(vec![CountryLinkRow {
            id: 7,
            canonical_slot: 70,
        }]);
        let complete = with(TableView::Complete(links.clone()));
        let available = rows::<tables::CountryLinks>(&complete).unwrap();
        assert_eq!(country_slot(&available, 7), Ok(Some(70)));
        assert_eq!(country_slot(&available, 0), Ok(None));
        assert_eq!(
            country_slot(&available, 8),
            Err("country 8 has no CountryLinks row".to_owned())
        );
        let truncated = with(TableView::Truncated {
            rows: links,
            missing: 3,
        });
        let available = rows::<tables::CountryLinks>(&truncated).unwrap();
        assert_eq!(country_slot(&available, 7), Ok(Some(70)));
        assert!(
            country_slot(&available, 8)
                .unwrap_err()
                .starts_with("the CountryLinks table is truncated this hour (3 rows not reached)")
        );
    }
}
