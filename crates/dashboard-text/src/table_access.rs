use crate::invalid;
use replay_data::{
    rows::{LexicalTokenRow, TableId, TableRows, TableSchema, column, mask, tables},
    scope::CaptureMode,
};
#[cfg(test)]
use replay_frame::frame::DecodedFrame;
use replay_frame::frame::{HourFrame, Rowset, TableView};
use std::{collections::BTreeMap, io};

pub enum Available<'a, T: TableSchema> {
    Complete(&'a [T::Row]),
    Truncated { rows: &'a [T::Row], missing: u32 },
    Unavailable { reason: &'a str },
    NotRecorded { mode: CaptureMode },
}

impl<'a, T: TableSchema> Available<'a, T> {
    pub fn rows(&self) -> &'a [T::Row] {
        match self {
            Available::Complete(rows) | Available::Truncated { rows, .. } => rows,
            Available::Unavailable { .. } | Available::NotRecorded { .. } => &[],
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, Available::Complete(_))
    }
}

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
        Available::Complete(_) => format!("country {country} has no country links row"),
        Available::Truncated { missing, .. } => format!(
            "the country links table is truncated this hour ({missing} rows not reached) and holds no row for country {country}"
        ),
        Available::Unavailable { reason } => format!(
            "the country links table is unavailable this hour ({reason}), so country {country} is unresolved"
        ),
        Available::NotRecorded { mode } => format!(
            "the country links table is not recorded in a {mode} recording, so country {country} is unresolved"
        ),
    })
}

pub fn rows<T: TableSchema>(
    frame: &dyn replay_frame::frame::DecodedSource,
) -> io::Result<Available<'_, T>> {
    match frame.view(T::ID.index()) {
        TableView::Complete(rows) => Ok(Available::Complete(typed::<T>(rows)?)),
        TableView::Truncated { rows, missing } => Ok(Available::Truncated {
            rows: typed::<T>(rows)?,
            missing: *missing,
        }),
        TableView::Unavailable { reason } => Ok(Available::Unavailable { reason }),
        TableView::NotRecorded { mode } => Ok(Available::NotRecorded { mode: *mode }),
    }
}

pub fn rows_fields<T: TableSchema>(
    frame: &dyn replay_frame::frame::DecodedSource,
    fields: u64,
) -> io::Result<Available<'_, T>> {
    replay_frame::frame::reading(fields, || rows::<T>(frame))
}

#[derive(Clone, Copy)]
pub struct TokenNames<'a> {
    rows: Rowset<'a, LexicalTokenRow>,
    complete: bool,
}

impl std::fmt::Debug for TokenNames<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenNames")
            .field("rows", &self.rows.len())
            .field("complete", &self.complete)
            .finish()
    }
}

const LITERAL: u64 = mask::<LexicalTokenRow>(&["literal"]);
const LITERAL_COLUMN: usize = column::<LexicalTokenRow>("literal");

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
            Self::Absent(token) => write!(f, "token {token} is not in lexical tokens"),
            Self::Missing(token) => {
                write!(f, "token {token} is among lexical tokens' missing rows")
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
            return Err(invalid(format!("{} holds rows of another kind", TableId::LexicalTokens.words())));
        }
        let rows = frame.lexical_rows();
        if !rows.is_sorted_by(|a, b| a.token < b.token) {
            return Err(invalid(format!("{} are not in order", TableId::LexicalTokens.words())));
        }
        Ok(Self {
            rows: Rowset::Decoded(rows.as_slice()),
            complete: frame.lexical_status().is_complete(),
        })
    }

    pub fn new_in(frame: HourFrame<'a>) -> io::Result<Self> {
        if let Some(decoded) = frame.decoded() {
            return Self::new(decoded.decoded_frame());
        }
        Ok(Self {
            rows: frame.held::<tables::LexicalTokens>(LITERAL),
            complete: frame.status(TableId::LexicalTokens).is_complete(),
        })
    }

    pub fn from_rows(rows: &'a [LexicalTokenRow], complete: bool) -> Self {
        Self {
            rows: Rowset::Decoded(rows),
            complete,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &'a str)> + '_ {
        self.rows
            .keys()
            .zip(self.rows.texts(|row| &row.literal, LITERAL_COLUMN))
            .filter_map(|(token, literal)| {
                let token = token as u32;
                let name = std::str::from_utf8(literal.ok()?).ok()?;
                (token != 0 && !name.contains('\0')).then_some((token, name))
            })
    }
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

    pub fn name(&self, token: u32) -> Result<&'a str, TokenError> {
        if token == 0 {
            return Err(TokenError::Rejected);
        }
        let literal = match self
            .rows
            .text(u128::from(token), |row| &row.literal, LITERAL_COLUMN)
        {
            Ok(Some(literal)) => literal,
            Ok(None) => {
                return Err(if self.complete {
                    TokenError::Absent(token)
                } else {
                    TokenError::Missing(token)
                });
            }
            Err(_) => return Err(TokenError::NotText(token)),
        };
        std::str::from_utf8(literal)
            .ok()
            .filter(|name| !name.contains('\0'))
            .ok_or(TokenError::NotText(token))
    }
}

pub type Refusals = BTreeMap<String, usize>;

pub fn refuse(refusals: &mut Refusals, reason: String) {
    *refusals.entry(reason).or_default() += 1;
}

pub use crate::triggers::fold_token;

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

    pub fn ambiguous(&self) -> impl Iterator<Item = &[String]> {
        self.0
            .values()
            .map(Vec::as_slice)
            .filter(|keys| keys.len() > 1)
    }

    pub fn keys(&self, spelling: &str) -> &[String] {
        self.0.get(&fold_token(spelling)).map_or(&[], Vec::as_slice)
    }
}

fn typed<T: TableSchema>(rows: &TableRows) -> io::Result<&[T::Row]> {
    rows.rows::<T>().ok_or_else(|| {
        invalid(format!("{} holds rows of another kind", T::ID.words()))
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
            "lexical tokens holds rows of another kind"
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
            "token 7 is among lexical tokens' missing rows"
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
        assert_eq!(keys.resolve("\u{e9}t\u{e9}"), Ok(Some("\u{e9}t\u{e9}")));
        assert_eq!(keys.resolve("\u{c9}t\u{c9}"), Ok(None));
    }

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
            Err("country 8 has no country links row".to_owned())
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
                .starts_with("the country links table is truncated this hour (3 rows not reached)")
        );
    }
}
