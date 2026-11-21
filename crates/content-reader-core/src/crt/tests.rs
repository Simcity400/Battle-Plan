use super::*;

#[test]
fn decimal_scan_does_not_detect_octal_or_hexadecimal() {
    assert_eq!(scan_d32(b"010"), Scanned::Stored(10));
    assert_eq!(scan_d32(b"0x10"), Scanned::Stored(0));
    assert_eq!(scan_d32(b"65536"), Scanned::Stored(65_536));
    assert_eq!(scan_d32(b"2147483648"), Scanned::Stored(i32::MIN));
    assert_eq!(scan_d32(b"999999999999999999999"), Scanned::Stored(-1));
    assert_eq!(scan_d32(b"bad"), Scanned::NoMatch);
    assert_eq!(scan_d32(b""), Scanned::End);
}
