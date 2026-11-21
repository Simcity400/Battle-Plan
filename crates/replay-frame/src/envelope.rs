
use replay_data::{FORMAT_VERSION, codec::invalid_static};
use std::io;

pub const RECORD_MAGIC: [u8; 4] = *b"H4SV";
pub const RECORD_HEADER_BYTES: usize = 4 + 2 + 4 + 4;
pub const MAX_RECORD_BYTES: usize = 1 << 30;

pub fn seal_record(mut out: Vec<u8>) -> io::Result<Vec<u8>> {
    let length = out
        .len()
        .checked_sub(RECORD_HEADER_BYTES)
        .ok_or_else(|| invalid_static("spool record lacks its envelope"))?;
    if length > MAX_RECORD_BYTES {
        return Err(invalid_static(
            "spool record exceeds the replay block limit",
        ));
    }
    let crc = crc32fast::hash(&out[RECORD_HEADER_BYTES..]);
    out[..4].copy_from_slice(&RECORD_MAGIC);
    out[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    out[6..10].copy_from_slice(&(length as u32).to_le_bytes());
    out[10..14].copy_from_slice(&crc.to_le_bytes());
    Ok(out)
}
