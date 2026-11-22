
use crate::ResolvedLayers;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::{fs::File, io, os::windows::io::AsRawHandle};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_BASIC_INFO, FileBasicInfo, GetFileInformationByHandleEx, SetFileInformationByHandle,
};

#[cfg(windows)]
pub(crate) fn file_timestamps(file: &File) -> (i64, i64) {
    let mut info = FILE_BASIC_INFO::default();
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&raw mut info).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        },
        0,
        "{}",
        io::Error::last_os_error()
    );
    (info.LastWriteTime, info.ChangeTime)
}

#[cfg(windows)]
pub(crate) fn set_file_timestamps(file: &File, modified: i64, changed: i64) {
    let info = FILE_BASIC_INFO {
        LastWriteTime: modified,
        ChangeTime: changed,
        ..Default::default()
    };
    assert_ne!(
        unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileBasicInfo,
                (&raw const info).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        },
        0,
        "{}",
        io::Error::last_os_error()
    );
}

#[cfg(windows)]
const FILETIME_TICKS_PER_SECOND: i64 = 10_000_000;

#[cfg(windows)]
pub fn rewrite_preserving_modified_time(path: &Path, bytes: &[u8]) {
    let original = File::open(path).unwrap();
    assert_eq!(original.metadata().unwrap().len(), bytes.len() as u64);
    let (modified, changed) = file_timestamps(&original);
    drop(original);
    std::fs::write(path, bytes).unwrap();
    let changed = changed + FILETIME_TICKS_PER_SECOND;
    let attributes = File::options().write(true).open(path).unwrap();
    set_file_timestamps(&attributes, modified, changed);
    drop(attributes);
    assert_eq!(file_timestamps(&File::open(path).unwrap()), (modified, changed));
}

#[cfg(all(test, windows))]
pub(crate) fn change_directory_with_new_stamp(path: &Path, change: impl FnOnce()) {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES,
    };

    let directory = File::options()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    let (_, changed) = file_timestamps(&directory);
    change();
    set_file_timestamps(&directory, 0, changed + FILETIME_TICKS_PER_SECOND);
}

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

pub struct Record<'a> {
    pub name: &'a [u8],
    pub host: u8,
    pub flags: u16,
    pub deflate: bool,
    pub data: &'a [u8],
}

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
