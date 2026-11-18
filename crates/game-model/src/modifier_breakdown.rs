/// Preorder node; child occurrences retain their native order, including aliases.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Node {
    pub children: u32,
    pub value: i64,
    pub name: Vec<u8>,
}

/// A complete raw source node. Playback selects keys and applies the recorded
/// definition metadata, including dynamic keys from installed content.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct SourceValue {
    pub key: u32,
    pub raw: i64,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct SourceNode {
    pub children: u32,
    pub name: Vec<u8>,
    pub values: Vec<SourceValue>,
}
