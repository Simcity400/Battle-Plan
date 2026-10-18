//! Shared synthetic frame wire fixture for frame and container tests.
use replay_data::rows::TableId;

pub fn empty_frame97(hour: i32, keyframe: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&hour.to_le_bytes());
    bytes.push(u8::from(keyframe));
    bytes.extend_from_slice(&10_u64.to_le_bytes());
    for &table in &TableId::ALL[..TableId::AvailableAdvisors.index()] {
        bytes.extend_from_slice(&(table as u64 + 1).to_le_bytes());
    }
    bytes.extend_from_slice(&20_u64.to_le_bytes());
    bytes.extend_from_slice(&30_u64.to_le_bytes());
    for _ in &TableId::ALL[..TableId::AvailableAdvisors.index()] {
        bytes.push(4);
        crate::codec::put_string(&mut bytes, "synthetic legacy unavailable").unwrap();
    }
    bytes
}
