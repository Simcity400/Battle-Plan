use serde_json::Value;

#[derive(Debug, Default)]
pub struct Compared {
    pub entries: u64,
    pub differing: u64,
    pub first: Option<String>,
    pub notes: Value,
}
