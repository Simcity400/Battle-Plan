use serde::{Deserialize, Serialize};
use std::{fmt, io};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueryError {
    Refused { text: String },
    Failed { message: String },
}

impl QueryError {
    pub fn context(self, context: &str) -> Self {
        match self {
            Self::Failed { message } => Self::Failed {
                message: format!("{context}: {message}"),
            },
            refused => refused,
        }
    }
}

impl From<io::Error> for QueryError {
    fn from(error: io::Error) -> Self {
        Self::from(&error)
    }
}

impl From<&io::Error> for QueryError {
    fn from(error: &io::Error) -> Self {
        match refusal(error) {
            Some(text) => Self::Refused { text: text.into() },
            None => Self::Failed {
                message: error.to_string(),
            },
        }
    }
}

impl From<QueryError> for io::Error {
    fn from(error: QueryError) -> Self {
        match error {
            QueryError::Refused { text } => refused(text),
            QueryError::Failed { message } => io::Error::other(message),
        }
    }
}

impl From<String> for QueryError {
    fn from(message: String) -> Self {
        Self::Failed { message }
    }
}

impl From<&str> for QueryError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Refused { text } => text,
            Self::Failed { message } => message,
        })
    }
}

impl std::error::Error for QueryError {}

#[derive(Debug)]
struct Refusal(String);

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

pub fn refused(text: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, Refusal(text.into()))
}

pub fn duplicate(error: &io::Error) -> io::Error {
    match refusal(error) {
        Some(text) => refused(text),
        None => io::Error::new(error.kind(), error.to_string()),
    }
}

pub fn refusal(error: &io::Error) -> Option<&str> {
    let mut inner: &(dyn std::error::Error + 'static) = error.get_ref()?;
    loop {
        if let Some(refusal) = inner.downcast_ref::<Refusal>() {
            return Some(&refusal.0);
        }
        inner = if let Some(error) = inner.downcast_ref::<io::Error>() {
            error.get_ref()?
        } else if let Some(error) = inner.downcast_ref::<std::sync::Arc<io::Error>>() {
            error.get_ref()?
        } else {
            inner.source()?
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_typed_refusals_are_public() {
        let error = refused("Synthetic data was not recorded");
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(refusal(&error), Some("Synthetic data was not recorded"));
        let wrapped = io::Error::new(error.kind(), std::sync::Arc::new(error));
        assert_eq!(refusal(&wrapped), Some("Synthetic data was not recorded"));
        assert_eq!(
            QueryError::from(wrapped),
            QueryError::Refused {
                text: "Synthetic data was not recorded".into()
            }
        );
        let error = io::Error::other("Synthetic data was not recorded");
        assert_eq!(refusal(&error), None);
        assert_eq!(
            QueryError::from(error),
            QueryError::Failed {
                message: "Synthetic data was not recorded".into()
            }
        );
    }
}
