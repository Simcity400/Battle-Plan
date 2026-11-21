#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Node {
    pub children: u32,
    pub value: i64,
    pub name: Vec<u8>,
}

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
