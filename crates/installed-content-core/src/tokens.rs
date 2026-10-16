//! Name identity for databases that register and find entries by lexer token.
//!
//! Empty names and names starting with a digit or `-` share one rejected token. Other
//! names match after lowering only ASCII `A`-`Z`, using FNV-1a and a case-folded byte
//! comparison. Equal folded names therefore share one token.
//!
//! Readers register token text only through its first NUL
//! ([`crate::parser::c_string`]). Database lookup scans entries in their stored order
//! after the null object and selects the first matching token, returning the null
//! object when none matches ([`first`]).

use crate::parser::c_string;

/// A name's token.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Token {
    /// Token 0, assigned to every rejected name.
    Rejected,
    /// The token of a registered name, as its folded bytes.
    Name(Vec<u8>),
}

impl Token {
    /// Register the whole `name` with lexer token identity.
    pub fn register(name: &[u8]) -> Self {
        match name.first() {
            None | Some(b'0'..=b'9' | b'-') => Self::Rejected,
            Some(_) => Self::Name(name.to_ascii_lowercase()),
        }
    }

    /// Register a token's C-string text up to its first NUL.
    pub fn c_str(text: &[u8]) -> Self {
        Self::register(c_string(text))
    }

    /// Find a registered name by token identity. Rejected names share the rejected
    /// token. Other names resolve only if a reader has already registered the folded
    /// name; return `None` when absent.
    pub fn existing(name: &[u8], registered: &[Token]) -> Option<Self> {
        match Self::register(c_string(name)) {
            Self::Rejected => Some(Self::Rejected),
            token if registered.contains(&token) => Some(token),
            _ => None,
        }
    }

    /// Whether this is token 0.
    pub fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected)
    }
}

/// Find the first entry in database order whose token matches, excluding the null
/// object. `None` represents that null object.
pub fn first<'a, T>(entries: impl IntoIterator<Item = (&'a Token, T)>, token: &Token) -> Option<T> {
    entries
        .into_iter()
        .find_map(|(entry, value)| (entry == token).then_some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty names and names starting with a digit or `-` share the rejected token;
    /// other names fold ASCII `A`-`Z` only. C strings end at NUL.
    #[test]
    fn names_register_as_the_lexer_registers_them() {
        for rejected in [&b""[..], b"1", b"-x", b"9abc", b"\0tail"] {
            assert_eq!(Token::c_str(rejected), Token::Rejected, "{rejected:?}");
        }
        assert_eq!(Token::register(b"Gun_1"), Token::register(b"gUN_1"));
        assert_ne!(
            Token::register("É".as_bytes()),
            Token::register("é".as_bytes())
        );
        assert_eq!(Token::c_str(b"slot\0rest"), Token::register(b"slot"));
        let tokens = [
            Token::register(b"a"),
            Token::Rejected,
            Token::register(b"A"),
        ];
        let entries = tokens.iter().zip(["a", "zero", "A again"]);
        assert_eq!(first(entries.clone(), &Token::register(b"a")), Some("a"));
        assert_eq!(first(entries.clone(), &Token::Rejected), Some("zero"));
        assert_eq!(first(entries, &Token::register(b"b")), None);
        let registered = [Token::register(b"oil")];
        assert_eq!(
            Token::existing(b"OIL", &registered),
            Some(Token::register(b"oil"))
        );
        assert_eq!(Token::existing(b"7", &registered), Some(Token::Rejected));
        assert_eq!(Token::existing(b"steel", &registered), None);
    }
}
