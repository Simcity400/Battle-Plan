use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key<'a>(&'a [u8]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidKey;

impl<'a> Key<'a> {
    pub fn new(varint: &'a [u8]) -> Result<Self, InvalidKey> {
        let Some((&last, rest)) = varint.split_last() else {
            return Err(InvalidKey);
        };
        if last & 0x80 != 0
            || (!rest.is_empty() && last == 0)
            || rest.iter().any(|byte| byte & 0x80 == 0)
        {
            return Err(InvalidKey);
        }
        Ok(Self(varint))
    }

    pub fn varint(self) -> &'a [u8] {
        self.0
    }
}

impl Ord for Key<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }
}

impl PartialOrd for Key<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Count<'a>(Key<'a>);

impl<'a> Count<'a> {
    pub fn new(varint: &'a [u8]) -> Result<Self, InvalidKey> {
        Key::new(varint).map(Self)
    }
    pub fn varint(self) -> &'a [u8] {
        self.0.varint()
    }
}
