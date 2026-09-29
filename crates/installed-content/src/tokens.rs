//! Names as the lexer's token registry identifies them, for the databases
//! that register and find their entries by token.
//!
//! CPdxLexer::add_dynamic_token (1424A6500 -> add_dynamic_token_impl
//! 1424A65A0) gives an empty name, and a name starting with a digit or `-`,
//! token 0 ("Trying to add empty dynamic token.", "keywords may not start
//! with a digit or '-'"). Any other name gets the token already registered
//! for a name equal to it under 1424A5EE0's fold (FNV-1a over the bytes with
//! only `A`-`Z` lowered, then `_strnicmp`), else a new one. So every rejected
//! name is the same token 0, and two names equal under the fold are one
//! token.
//!
//! The readers hand it a C string: CReader::ReadTokenId (1424ABA00) and the
//! loops that register a key or value (`strlen` of the key slot +0x38 or the
//! value slot +0xC8, e.g. 140C89CC0's `default_modules`, 140C8C260's slots,
//! 1409E5170's `duplicate_archetypes`, load_file 140AB2ED0) stop at the
//! text's first NUL ([`crate::parser::c_string`]).
//!
//! A database that finds an entry by token (GetModuleByToken 1409EB610,
//! 140712DD0, 140AB2920) scans its entries from the first after the null
//! object and returns the first whose token matches, else the null object:
//! [`first`].

use crate::parser::c_string;

/// A name's token.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Token {
    /// Token 0, which add_dynamic_token gives every name it rejects.
    Rejected,
    /// The token of a registered name, as its folded bytes.
    Name(Vec<u8>),
}

impl Token {
    /// add_dynamic_token (1424A65A0) on `name`, whole.
    pub fn register(name: &[u8]) -> Self {
        match name.first() {
            None | Some(b'0'..=b'9' | b'-') => Self::Rejected,
            Some(_) => Self::Name(name.to_ascii_lowercase()),
        }
    }

    /// add_dynamic_token on a token's text as a C string: up to its first
    /// NUL (ReadTokenId 1424ABA00 and the registering loops).
    pub fn c_str(text: &[u8]) -> Self {
        Self::register(c_string(text))
    }

    /// FindExistingDynamicTokenFromString (1424A6FF0 ->
    /// FindTokenNoCaseOrDefault 14022EC40) on a name `registered` does not
    /// hold: the lexer never holds a rejected name, so that is token 0;
    /// any other is the token the lexer holds for it, or 0 when it holds
    /// none, which depends on every name any reader registered. `None` for
    /// that case.
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

/// The first of `entries` (in the database's order, the null object left
/// out) whose token is `token`: GetModuleByToken (1409EB610) and its
/// siblings. `None` is the null object.
pub fn first<'a, T>(entries: impl IntoIterator<Item = (&'a Token, T)>, token: &Token) -> Option<T> {
    entries
        .into_iter()
        .find_map(|(entry, value)| (entry == token).then_some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1424A65A0: empty names and names starting with a digit or `-` are
    /// all token 0; any other name folds `A`-`Z` only; a C string ends at
    /// its NUL.
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
