//! Build-specific parts that verify themselves: each is available or withheld by name.

use std::{borrow::Cow, fmt, io};

/// A build-specific part that failed its own check, with the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Withheld {
    pub part: Cow<'static, str>,
    pub reason: String,
}

/// A part that passed its own check, or the reason it is withheld.
pub type Verified<T> = Result<T, Withheld>;

impl Withheld {
    pub fn new(part: impl Into<Cow<'static, str>>, reason: impl fmt::Display) -> Self {
        Self {
            part: part.into(),
            reason: reason.to_string(),
        }
    }

    /// `self`, naming `part` as what it withholds because `self` is missing.
    pub fn requires(&self, part: impl Into<Cow<'static, str>>) -> Self {
        Self {
            part: part.into(),
            reason: format!("requires {}, which is withheld ({})", self.part, self.reason),
        }
    }
}

impl fmt::Display for Withheld {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} withheld: {}", self.part, self.reason)
    }
}

impl std::error::Error for Withheld {}

impl From<Withheld> for io::Error {
    fn from(withheld: Withheld) -> Self {
        io::Error::new(io::ErrorKind::Unsupported, withheld)
    }
}

impl From<&Withheld> for io::Error {
    fn from(withheld: &Withheld) -> Self {
        withheld.clone().into()
    }
}

/// The withheld part an I/O error carries, if it is one.
pub fn withheld_of(error: &io::Error) -> Option<&Withheld> {
    error.get_ref()?.downcast_ref::<Withheld>()
}
