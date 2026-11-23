use super::*;
use engine_native_memory::memory::SliceMemory;

#[test]
fn doctrine_preserves_signed_cache_and_requires_country_backlink() {
    let mut bytes = vec![0; 0x4000];
    bytes[0x1000 + ccountry::TECHNOLOGY_STATUS..0x1008 + ccountry::TECHNOLOGY_STATUS]
        .copy_from_slice(&0x3000_u64.to_le_bytes());
    bytes[0x3000..0x3008].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[0x3000 + COUNTRY..0x3008 + COUNTRY].copy_from_slice(&0x1000_u64.to_le_bytes());
    bytes[0x3000 + LAND_DOCTRINE_LEVEL..0x3004 + LAND_DOCTRINE_LEVEL]
        .copy_from_slice(&i32::MIN.to_le_bytes());
    assert_eq!(
        land_doctrine_level(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            0x1000
        )
        .unwrap(),
        i32::MIN
    );
    bytes[0x3000 + COUNTRY] = 1;
    assert!(
        land_doctrine_level(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            0x1000
        )
        .unwrap_err()
        .contains("backlink")
    );
}
