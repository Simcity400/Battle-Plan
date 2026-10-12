use std::fmt;

/// A query refusal has a stable public name and a contextual explanation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WireError {
    pub name: &'static str,
    pub detail: String,
}

impl WireError {
    pub fn new(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.detail)
    }
}
impl std::error::Error for WireError {}
pub type Result<T> = std::result::Result<T, WireError>;

pub(crate) fn err<T>(name: &'static str, detail: impl Into<String>) -> Result<T> {
    Err(WireError::new(name, detail))
}

pub(crate) fn usize_count(value: u64) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_| WireError::new("size-overflow", "count does not fit this reader"))
}

pub(crate) fn checked_size(count: u64, width: u64) -> Result<usize> {
    usize_count(
        count
            .checked_mul(width)
            .ok_or_else(|| WireError::new("size-overflow", "buffer byte length overflow"))?,
    )
}

pub(crate) fn count32(value: usize) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| WireError::new("size-overflow", "reply-local count needs another unit"))
}

pub(crate) fn align8(value: usize) -> Result<usize> {
    value
        .checked_add(7)
        .map(|v| v & !7)
        .ok_or_else(|| WireError::new("size-overflow", "alignment overflow"))
}
