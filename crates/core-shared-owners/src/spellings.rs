use crate::{Change, Dated, Hour, Interval, Lookup, Owner};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spelling<'a> {
    pub literal: &'a [u8],
    pub content: ContentId,
}

pub trait Spellings: Owner {
    fn spelling(&self, token: Token, hour: Hour) -> io::Result<Lookup<'_, Dated<Spelling<'_>>>>;

    fn changes(
        &self,
        hours: Interval,
        visit: &mut dyn FnMut(Change<Token>) -> io::Result<()>,
    ) -> io::Result<()>;
}
