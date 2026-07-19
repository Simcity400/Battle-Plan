use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, ContentParseError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentParseError {
    pub message: String,
}

impl ContentParseError {
    pub fn parse(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ContentParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ContentParseError {}

impl From<io::Error> for ContentParseError {
    fn from(value: io::Error) -> Self {
        Self::parse(value.to_string())
    }
}
