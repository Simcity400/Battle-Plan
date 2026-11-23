use super::*;
use engine_native_parts::cequipment_type;
use engine_native_memory::memory::SliceMemory;

const LINE: usize = 0x100;
const VARIANT: usize = 0x400;
const TYPE: usize = 0x1000;

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; TYPE + 0x600];
    bytes[LINE + LINE_CONVERSION] = 1;
    for (address, value) in [
        (LINE + LINE_VARIANT, VARIANT as u64),
        (VARIANT + cequipment_variant::DEFINITION, TYPE as u64),
        (VARIANT + cequipment_variant::PARENT, 0x9_0000_0046),
    ] {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn converting_in(bytes: &[u8]) -> Result<bool, &'static str> {
    converting(&SliceMemory { base: 0, bytes }, LINE)
}

#[test]
fn conversion_needs_the_control_and_a_conversion_target() {
    assert_eq!(converting_in(&fixture()), Ok(true));
    let mut bytes = fixture();
    bytes[LINE + LINE_CONVERSION] = 0;
    assert_eq!(converting_in(&bytes), Ok(false));
    let mut bytes = fixture();
    bytes[VARIANT + cequipment_variant::ORIGIN_LINK..][..4].copy_from_slice(&7_i32.to_le_bytes());
    assert_eq!(converting_in(&bytes), Ok(false));
    bytes[TYPE + cequipment_type::CONVERT_FROM_COUNT..][..4].copy_from_slice(&1_i32.to_le_bytes());
    assert_eq!(converting_in(&bytes), Ok(true));
    let mut bytes = fixture();
    bytes[VARIANT + cequipment_variant::PARENT..][..8].fill(0);
    assert_eq!(converting_in(&bytes), Ok(false));
    bytes[TYPE + cequipment_type::CONVERSION_FAMILY] = 1;
    assert_eq!(converting_in(&bytes), Ok(true));
    let mut bytes = fixture();
    bytes[LINE + LINE_VARIANT..][..8].fill(0);
    assert_eq!(converting_in(&bytes), Ok(false));
}

#[test]
fn unreadable_conversion_operands_are_refused_not_new() {
    let mut bytes = fixture();
    assert!(converting_in(&bytes[..LINE + LINE_CONVERSION]).is_err());
    bytes[VARIANT + cequipment_variant::PARENT..][..8].fill(0);
    assert!(converting_in(&bytes[..TYPE]).is_err());
    let mut null_type = fixture();
    null_type[VARIANT + cequipment_variant::DEFINITION..][..8].fill(0);
    assert!(converting_in(&null_type).is_err());
}
