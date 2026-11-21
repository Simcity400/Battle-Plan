
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub arrays: [u64; 2],
    pub rows: [u64; 2],
    pub spans: [u64; 2],
}
