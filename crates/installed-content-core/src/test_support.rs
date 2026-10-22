//! Synthetic content-layer fixtures for reader tests.

use crate::ResolvedLayers;
use std::path::{Path, PathBuf};

/// A base game folder and a mod folder under a fresh temporary root.
pub struct Stack {
    root: PathBuf,
}

impl Stack {
    pub fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4_mod_layers_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        Self { root }
    }

    pub fn base(&self) -> PathBuf {
        self.root.join("base")
    }

    pub fn addon(&self) -> PathBuf {
        self.root.join("mod")
    }

    pub fn write(&self, layer: &Path, path: &str, text: &str) {
        let path = layer.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    pub fn layers(&self, replace_paths: &[&str]) -> ResolvedLayers {
        let (base, addon) = (self.base(), self.addon());
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&addon).unwrap();
        ResolvedLayers::from_folders(&[(&base, &[]), (&addon, replace_paths)]).unwrap()
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// One record of [`zip_records`].
pub struct Record<'a> {
    pub name: &'a [u8],
    /// The high byte of its version-made-by.
    pub host: u8,
    /// Its general-purpose flags (`0x800`: the name is UTF-8).
    pub flags: u16,
    /// Raw deflate (method 8) rather than stored.
    pub deflate: bool,
    pub data: &'a [u8],
}

/// A ZIP written byte by byte after `prefix` (a self-extractor's stub, which every offset in
/// the archive leaves out).
pub fn zip_records(prefix: &[u8], records: &[Record]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let (mut file, mut central) = (Vec::new(), Vec::new());
    for record in records {
        let offset = file.len() as u32;
        let stored = if record.deflate {
            miniz_oxide::deflate::compress_to_vec(record.data, 6)
        } else {
            record.data.to_vec()
        };
        let method: u16 = if record.deflate { 8 } else { 0 };
        let (crc, length) = (crc32(record.data), record.name.len() as u16);
        let (compressed, size) = (stored.len() as u32, record.data.len() as u32);
        let mut fields = vec![20, 0];
        fields.extend(record.flags.to_le_bytes());
        fields.extend(method.to_le_bytes());
        fields.extend([0; 4]);
        fields.extend(crc.to_le_bytes());
        fields.extend(compressed.to_le_bytes());
        fields.extend(size.to_le_bytes());
        fields.extend(length.to_le_bytes());
        file.extend(0x0403_4B50_u32.to_le_bytes());
        file.extend(&fields);
        file.extend([0, 0]);
        file.extend(record.name);
        file.extend(&stored);
        central.extend(0x0201_4B50_u32.to_le_bytes());
        central.extend([20, record.host]);
        central.extend(&fields);
        central.extend([0; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(record.name);
    }
    let start = file.len() as u32;
    let count = (records.len() as u16).to_le_bytes();
    file.extend(&central);
    file.extend(0x0605_4B50_u32.to_le_bytes());
    file.extend([0, 0, 0, 0, count[0], count[1], count[0], count[1]]);
    file.extend((central.len() as u32).to_le_bytes());
    file.extend(start.to_le_bytes());
    file.extend([0, 0]);
    [prefix.to_vec(), file].concat()
}
