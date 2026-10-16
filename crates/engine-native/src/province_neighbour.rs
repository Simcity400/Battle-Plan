//! Province-definition neighbour entry.

/// Neighbour-entry stride.
pub const STRIDE: usize = 0x30;
/// Neighbour signed i32 province ID.
pub const PROVINCE_ID: usize = 0x08;
/// Adjacency crossing-kind byte.
pub const CROSSING_KIND: usize = 0x10;
/// Adjacency flag byte, independent of crossing kind.
pub const FLAG: usize = 0x11;
/// Signed fixed5 movement distance.
pub const MOVEMENT_DISTANCE: usize = 0x18;
/// Crossing point as signed i32 x/y, copied with kind, flag and distance to the reverse entry.
pub const CROSSING_POINT: usize = 0x20;

/// Every field one entry stores for movement and the plan builders, bit for
/// bit, decoded from an owned copy of the whole entry.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub id: u32,
    pub crossing_kind: u8,
    /// Raw +0x11 byte; nonzero suppresses the movement-arrow midpoint.
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

/// One ordered adjacency observation. Distance and crossing are read only for
/// a matching entry, so an unread later entry cannot fail an earlier match.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeometryFields {
    pub id: Option<Result<u32, &'static str>>,
    pub distance: Option<Result<i64, &'static str>>,
    pub crossing: Option<Result<u8, &'static str>>,
    pub next: Option<usize>,
}
