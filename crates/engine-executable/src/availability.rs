
use std::{borrow::Cow, fmt, io};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Withheld {
    pub part: Cow<'static, str>,
    pub reason: String,
}

pub type Verified<T> = Result<T, Withheld>;

impl Withheld {
    pub fn new(part: impl Into<Cow<'static, str>>, reason: impl fmt::Display) -> Self {
        Self {
            part: part.into(),
            reason: reason.to_string(),
        }
    }

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

pub fn withheld_of(error: &io::Error) -> Option<&Withheld> {
    error.get_ref()?.downcast_ref::<Withheld>()
}
