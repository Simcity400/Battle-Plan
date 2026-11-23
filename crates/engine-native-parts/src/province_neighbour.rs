
pub const STRIDE: usize = 0x30;
pub const PROVINCE_ID: usize = 0x08;
pub const CROSSING_KIND: usize = 0x10;
pub const FLAG: usize = 0x11;
pub const MOVEMENT_DISTANCE: usize = 0x18;
pub const CROSSING_POINT: usize = 0x20;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub id: u32,
    pub crossing_kind: u8,
    pub flag: u8,
    pub distance: i64,
    pub crossing: [i32; 2],
}

impl Record {
    pub fn decode(entry: &[u8; STRIDE]) -> Self {
        let word = |at: usize| u32::from_le_bytes(entry[at..at + 4].try_into().unwrap());
        Self {
            id: word(PROVINCE_ID),
            crossing_kind: entry[CROSSING_KIND],
            flag: entry[FLAG],
            distance: i64::from_le_bytes(
                entry[MOVEMENT_DISTANCE..MOVEMENT_DISTANCE + 8]
                    .try_into()
                    .unwrap(),
            ),
            crossing: [word(CROSSING_POINT) as i32, word(CROSSING_POINT + 4) as i32],
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GeometryFields {
    pub id: Option<Result<u32, &'static str>>,
    pub distance: Option<Result<i64, &'static str>>,
    pub crossing: Option<Result<u8, &'static str>>,
    pub next: Option<usize>,
}
