use std::time::Duration;

pub fn test_tga_rgba8(width: u16, height: u16) -> Vec<u8> {
    let mut tga = vec![0; 18 + usize::from(width) * usize::from(height) * 4];
    tga[2] = 2;
    tga[12..14].copy_from_slice(&width.to_le_bytes());
    tga[14..16].copy_from_slice(&height.to_le_bytes());
    tga[16] = 32;
    tga[17] = 8;
    tga
}

pub const TEST_HANG_GUARD: Duration = Duration::from_secs(120);

pub mod image;
