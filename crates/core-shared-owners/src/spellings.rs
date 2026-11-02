use crate::{Change, Dated, Hour, Interval, Lookup, Owner};
use std::io;

/// A LexicalTokens token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token(pub u32);

/// The identity of a literal's content: equal exactly where the literals are, so caches built
/// from a literal (its localisation, its display form) are keyed by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentId(pub u64);

/// A token's spelling: its recorded literal and that literal's content identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spelling<'a> {
    pub literal: &'a [u8],
    pub content: ContentId,
}

/// LexicalTokens spellings, keyed by token. No family wakes on the whole table: each reads the
/// tokens it reached and follows their changes.
pub trait Spellings: Owner {
    fn spelling(&self, token: Token, hour: Hour) -> io::Result<Lookup<'_, Dated<Spelling<'_>>>>;

    /// The tokens whose spellings changed in `hours`, in hour order.
    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<Token>) -> io::Result<()>,
    ) -> io::Result<()>;
}
