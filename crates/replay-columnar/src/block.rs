
pub(crate) fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub(crate) fn zig(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

pub(crate) fn unzig(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}
