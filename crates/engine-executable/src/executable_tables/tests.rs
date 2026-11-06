use super::pe::PREFERRED_IMAGE_BASE;
use super::{PeImage, TableDigestMismatch, verify_digest};
use crate::build::ExecutableIdentity;
use std::io;

const OPTIONAL: usize = 0x98;
const SECTIONS: usize = OPTIONAL + 0xf0;
const DIRECTORIES: usize = OPTIONAL + 112;
const TEXT_FILE: usize = 0x200;
const DATA_FILE: usize = 0x400;

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x600];
    bytes[..2].copy_from_slice(b"MZ");
    put_u32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    put_u16(&mut bytes, 0x84, 0x8664);
    put_u16(&mut bytes, 0x86, 2);
    put_u16(&mut bytes, 0x94, 0xf0);
    put_u16(&mut bytes, OPTIONAL, 0x20b);
    put_u64(&mut bytes, OPTIONAL + 24, PREFERRED_IMAGE_BASE);
    put_u32(&mut bytes, OPTIONAL + 32, 0x1000);
    put_u32(&mut bytes, OPTIONAL + 36, 0x200);
    put_u32(&mut bytes, OPTIONAL + 56, 0x3000);
    put_u32(&mut bytes, OPTIONAL + 60, 0x200);
    put_u32(&mut bytes, OPTIONAL + 108, 16);
    put_u32(&mut bytes, DIRECTORIES + 3 * 8, 0x2000);
    put_u32(&mut bytes, DIRECTORIES + 3 * 8 + 4, 12);
    put_u32(&mut bytes, SECTIONS + 8, 0x300);
    put_u32(&mut bytes, SECTIONS + 12, 0x1000);
    put_u32(&mut bytes, SECTIONS + 16, 0x200);
    put_u32(&mut bytes, SECTIONS + 20, TEXT_FILE as u32);
    put_u32(&mut bytes, SECTIONS + 36, 0x6000_0020);
    put_u32(&mut bytes, SECTIONS + 40 + 8, 0x200);
    put_u32(&mut bytes, SECTIONS + 40 + 12, 0x2000);
    put_u32(&mut bytes, SECTIONS + 40 + 16, 0x200);
    put_u32(&mut bytes, SECTIONS + 40 + 20, DATA_FILE as u32);
    put_u32(&mut bytes, SECTIONS + 40 + 36, 0x4000_0040);
    bytes[TEXT_FILE..TEXT_FILE + 3].copy_from_slice(&[0x90, 0x90, 0xc3]);
    put_u32(&mut bytes, DATA_FILE, 0x1000);
    put_u32(&mut bytes, DATA_FILE + 4, 0x1003);
    put_u32(&mut bytes, DATA_FILE + 8, 0x2080);
    bytes[DATA_FILE + 0x80] = 1;
    bytes
}

fn invalid_image(bytes: Vec<u8>) {
    assert_eq!(
        PeImage::parse(bytes).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn malformed_headers_and_truncated_sections_are_refused() {
    invalid_image(Vec::new());
    let mut bytes = fixture();
    bytes[0] = b'X';
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, 0x3c, u32::MAX);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u16(&mut bytes, 0x84, 0x014c);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u16(&mut bytes, OPTIONAL, 0x10b);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u64(&mut bytes, OPTIONAL + 24, PREFERRED_IMAGE_BASE + 0x1000);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u16(&mut bytes, 0x86, 20);
    invalid_image(bytes);
    let mut bytes = fixture();
    bytes.truncate(SECTIONS + 40);
    invalid_image(bytes);
}

#[test]
fn raw_backing_controls_reads_even_when_virtual_sections_are_larger() {
    let image = PeImage::parse(fixture()).unwrap();
    assert_eq!(image.bytes(0x1000, 3).unwrap(), &[0x90, 0x90, 0xc3]);
    assert!(image.bytes(0x1200, 1).is_err());
    assert!(image.bytes(0x11ff, 2).is_err());
    assert!(image.bytes(0x1000, usize::MAX).is_err());
    assert!(image.bytes(u32::MAX, 2).is_err());
    assert!(image.bytes(0x100, 1).is_err());
    assert!(image.bytes(0x11ff, 0x1002).is_err());
}

#[test]
fn section_overflows_truncation_and_overlaps_are_refused() {
    let mut bytes = fixture();
    put_u32(&mut bytes, SECTIONS + 12, u32::MAX - 1);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, SECTIONS + 20, 0x500);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, SECTIONS + 40 + 12, 0x1100);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, SECTIONS + 40 + 20, 0x300);
    invalid_image(bytes);
}

#[test]
fn directories_must_fit_the_header_and_their_file_backing() {
    let mut bytes = fixture();
    put_u32(&mut bytes, OPTIONAL + 108, u32::MAX);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, DIRECTORIES + 8, 0x2000);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, DIRECTORIES + 8, 0x21ff);
    put_u32(&mut bytes, DIRECTORIES + 12, 2);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, DIRECTORIES + 3 * 8 + 4, 13);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, DIRECTORIES + 4 * 8, 0x5ff);
    put_u32(&mut bytes, DIRECTORIES + 4 * 8 + 4, 2);
    invalid_image(bytes);
}

#[test]
fn certificate_directory_uses_file_offsets() {
    let mut bytes = fixture();
    put_u32(&mut bytes, DIRECTORIES + 4 * 8, 0x580);
    put_u32(&mut bytes, DIRECTORIES + 4 * 8 + 4, 0x80);
    PeImage::parse(bytes).unwrap();
}

#[test]
fn function_extents_require_executable_backing_and_order() {
    let image = PeImage::parse(fixture()).unwrap();
    assert_eq!(image.functions(), &[(0x1000, 0x1003)]);
    assert_eq!(image.function(0x1001).unwrap(), (0x1000, 0x1003));
    assert!(image.function(0x1003).is_err());
    let mut bytes = fixture();
    put_u32(&mut bytes, DATA_FILE + 4, 0x1201);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, DATA_FILE + 8, 0x2300);
    invalid_image(bytes);
    let mut bytes = fixture();
    put_u32(&mut bytes, DIRECTORIES + 3 * 8 + 4, 24);
    put_u32(&mut bytes, DATA_FILE + 12, 0x1002);
    put_u32(&mut bytes, DATA_FILE + 16, 0x1005);
    put_u32(&mut bytes, DATA_FILE + 20, 0x2080);
    invalid_image(bytes);
}

#[test]
fn strings_require_exact_utf8_and_a_nul_in_the_same_section() {
    let mut bytes = fixture();
    bytes[TEXT_FILE + 0x20..TEXT_FILE + 0x27].copy_from_slice(b"sample\0");
    let image = PeImage::parse(bytes).unwrap();
    assert_eq!(image.string(0x1020).unwrap(), "sample");
    let mut bytes = fixture();
    bytes[TEXT_FILE + 0x20..TEXT_FILE + 0x22].copy_from_slice(&[0xff, 0]);
    assert!(PeImage::parse(bytes).unwrap().string(0x1020).is_err());
    let mut bytes = fixture();
    bytes[TEXT_FILE + 0x20..DATA_FILE].fill(b'x');
    assert!(PeImage::parse(bytes).unwrap().string(0x1020).is_err());
}

#[test]
fn instruction_ips_use_preferred_addresses_and_invalid_spans_are_refused() {
    let image = PeImage::parse(fixture()).unwrap();
    let instructions = image.instructions(0x1000, 0x1003).unwrap();
    assert_eq!(instructions.len(), 3);
    assert_eq!(instructions[0].ip(), PREFERRED_IMAGE_BASE + 0x1000);
    assert_eq!(image.va_to_rva(instructions[2].ip()).unwrap(), 0x1002);
    assert!(image.va_to_rva(PREFERRED_IMAGE_BASE - 1).is_err());
    assert!(image.va_to_rva(PREFERRED_IMAGE_BASE + 0x3000).is_err());
    assert!(image.instructions(0x2000, 0x2001).is_err());
    assert!(image.instructions(0x1001, 0x1000).is_err());
    let mut bytes = fixture();
    bytes[DATA_FILE - 1] = 0x0f;
    assert!(
        PeImage::parse(bytes)
            .unwrap()
            .instructions(0x11ff, 0x1200)
            .is_err()
    );
}

#[test]
fn digest_errors_preserve_the_table_identity_and_actual_hash() {
    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    verify_digest("synthetic table", b"", EMPTY).unwrap();
    let error = verify_digest("synthetic table", b"changed", EMPTY).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let mismatch = error
        .get_ref()
        .unwrap()
        .downcast_ref::<TableDigestMismatch>()
        .unwrap();
    assert_eq!(mismatch.table, "synthetic table");
    assert_eq!(mismatch.bytes, 7);
    assert_eq!(
        mismatch.actual_sha256,
        ExecutableIdentity::of(b"changed").sha256
    );
    assert_eq!(
        verify_digest("synthetic table", b"", "invalid")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
}
