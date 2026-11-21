#[inline]
pub fn is_canonical_range(addr: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = addr.checked_add(len.wrapping_sub(1)) else {
        return false;
    };
    let high = addr >> 47;
    matches!(high, 0 | 0x1FFFF) && end >> 47 == high
}
