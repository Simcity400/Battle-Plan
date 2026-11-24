
use engine_native_memory::{
    Memory,
    capture_text::{Owned, Value},
    cpdx_array::Array,
    msvc_string::{STRIDE, Text},
    msvc_tree_node as tree_node,
    read,
    walk::Outcome,
};
use crate::{
    dlc_content_entry as content_entry,
    dlc_content_map_node as content_node,
    msvc_map,
};
use engine_sites::Rva;

pub const SINGLETON_RVA: Rva = Rva(0x034f_83a8);
pub const SEARCH_PATH_RVA: Rva = Rva(0x0369_17a8);
pub const DLC_FILTER_STRINGS: usize = 0x50;
pub const REQUESTED_MODS: usize = 0x68;
pub const DLCS: usize = 0xa0;
pub const MODS: usize = 0xb0;

pub type Descriptor = Text<1024>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Entry<N = Text<256>, D = Descriptor> {
    pub name: N,
    pub descriptor: D,
    pub reported_active: bool,
}

pub struct Buffers<'a, N = Text<256>, D = Descriptor> {
    pub dlc_filter_strings: &'a mut [D],
    pub requested_mods: &'a mut [D],
    pub dlcs: &'a mut [Entry<N, D>],
    pub mods: &'a mut [Entry<N, D>],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub dlc_filter_strings: Outcome,
    pub requested_mods: Outcome,
    pub dlcs: Outcome,
    pub mods: Outcome,
}

impl Observation {
    fn unavailable(reason: &'static str) -> Self {
        let outcome = Outcome::Unavailable { reason };
        Self {
            dlc_filter_strings: outcome,
            requested_mods: outcome,
            dlcs: outcome,
            mods: outcome,
        }
    }
}

pub fn observe(memory: &impl Memory, base: usize, buffers: Buffers<'_>) -> Observation {
    observe_inner(memory, base, buffers, false)
}

pub fn prepare(
    memory: &impl Memory,
    base: usize,
    buffers: Buffers<'_, Owned, Owned>,
) -> Observation {
    observe_inner(memory, base, buffers, true)
}

pub fn observe_prepared(
    memory: &impl Memory,
    base: usize,
    buffers: Buffers<'_, Owned, Owned>,
) -> Observation {
    observe_inner(memory, base, buffers, false)
}

fn observe_inner<N: Value, D: Value>(
    memory: &impl Memory,
    base: usize,
    buffers: Buffers<'_, N, D>,
    growing: bool,
) -> Observation {
    let Some(manager) = engine_sites::at(base, SINGLETON_RVA)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
    else {
        return Observation::unavailable("DLC/mod manager is unreadable or not constructed");
    };
    let dlc_filter_strings = read_strings_inner(
        memory,
        manager,
        DLC_FILTER_STRINGS,
        buffers.dlc_filter_strings,
        growing,
    );
    let requested_mods = read_strings_inner(
        memory,
        manager,
        REQUESTED_MODS,
        buffers.requested_mods,
        growing,
    );
    let dlcs = walk_map_inner(memory, manager, DLCS, buffers.dlcs, growing, |value, _| {
        read::<u8>(memory, field(value, content_entry::DLC_REPORTED_ACTIVE)?)
            .map(|v| v != 0)
            .ok_or("DLC active byte unreadable")
    });
    let mods = match requested_mods {
        Outcome::Rows { len } => walk_map_inner(
            memory,
            manager,
            MODS,
            buffers.mods,
            growing,
            |_, descriptor| {
                Ok(buffers.requested_mods[..len].iter().any(|requested| {
                    requested_matches(requested.as_bytes(), descriptor.as_bytes())
                }))
            },
        ),
        _ => Outcome::Unavailable {
            reason: "complete requested mod list unavailable",
        },
    };
    Observation {
        dlc_filter_strings,
        requested_mods,
        dlcs,
        mods,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub dlc_filter_strings: usize,
    pub requested_mods: usize,
    pub dlcs: usize,
    pub mods: usize,
    pub mounts: usize,
}

pub fn observe_counts(memory: &impl Memory, base: usize) -> Counts {
    let manager = engine_sites::at(base, SINGLETON_RVA)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0);
    let strings = |offset: usize| {
        manager
            .and_then(|m| m.checked_add(offset))
            .and_then(|header| Array::read(memory, header, STRIDE))
            .map_or(0, |array| array.count as usize)
    };
    let map = |offset: usize| {
        manager
            .and_then(|m| m.checked_add(offset + msvc_map::COUNT))
            .and_then(|count| read::<u64>(memory, count))
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0)
    };
    let mounts = (|| {
        let root = read::<u64>(memory, engine_sites::at(base, SEARCH_PATH_RVA)?)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)?;
        let mut index = 0_usize;
        loop {
            let address = root.checked_add(index.checked_mul(8)?)?;
            if read::<u64>(memory, address)? == 0 {
                return Some(index);
            }
            index = index.checked_add(1)?;
        }
    })()
    .unwrap_or(0);
    Counts {
        dlc_filter_strings: strings(DLC_FILTER_STRINGS),
        requested_mods: strings(REQUESTED_MODS),
        dlcs: map(DLCS),
        mods: map(MODS),
        mounts,
    }
}

pub fn observe_search_path(
    memory: &impl Memory,
    base: usize,
    output: &mut [Descriptor],
) -> Outcome {
    search_path_inner(memory, base, output, |memory, path, row| {
        *row = read_c_string(memory, path)?;
        Ok(())
    })
}

pub fn observe_search_path_owned(
    memory: &impl Memory,
    base: usize,
    output: &mut [Owned],
    growing: bool,
) -> Outcome {
    search_path_inner(memory, base, output, |memory, path, row| {
        let mut index = 0_usize;
        loop {
            let byte = path
                .checked_add(index)
                .and_then(|at| read::<u8>(memory, at))
                .ok_or("search path string unreadable")?;
            if byte == 0 {
                row.len = index;
                return Ok(());
            }
            if index == row.bytes.len() {
                if !growing {
                    return Err("search path string exceeds prepared native length");
                }
                crate::allocation::reserve(&mut row.bytes, 1)
                    .map_err(|_| "search path string allocation failed")?;
                row.bytes.push(byte);
            } else {
                row.bytes[index] = byte;
            }
            index = index
                .checked_add(1)
                .ok_or("search path string length overflow")?;
        }
    })
}

fn search_path_inner<M: Memory, T>(
    memory: &M,
    base: usize,
    output: &mut [T],
    mut copy: impl FnMut(&M, usize, &mut T) -> Result<(), &'static str>,
) -> Outcome {
    let result = (|| {
        let root_address =
            engine_sites::at(base, SEARCH_PATH_RVA).ok_or("search path root address withheld")?;
        let root = read::<u64>(memory, root_address)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("search path cache unreadable or not constructed")?;
        let mut index = 0_usize;
        loop {
            let address = root
                .checked_add(
                    index
                        .checked_mul(8)
                        .ok_or("search path list address overflow")?,
                )
                .ok_or("search path list address overflow")?;
            let path = read::<u64>(memory, address)
                .and_then(|p| usize::try_from(p).ok())
                .ok_or("search path entry pointer unreadable")?;
            if path == 0 {
                let len = index.min(output.len());
                return Ok(if len == index {
                    Outcome::Rows { len }
                } else {
                    Outcome::Truncated {
                        len,
                        unread_rows: u32::try_from(index - len)
                            .map_err(|_| "search path unread count overflow")?,
                    }
                });
            }
            if let Some(row) = output.get_mut(index) {
                copy(memory, path, row)?;
            }
            index = index
                .checked_add(1)
                .ok_or("search path list address overflow")?;
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub fn mounts_match<T: Value>(observed: &[T], expected: &[String]) -> bool {
    mount_difference(observed, expected).is_none()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountDifference<'a> {
    NoneExpected,
    Missing { position: usize, expected: &'a str },
    Extra { position: usize, observed: &'a [u8] },
    Differs {
        position: usize,
        observed: &'a [u8],
        expected: &'a str,
        folders: Folders,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Folders {
    Unreadable,
    Unopened { observed: bool, expected: bool },
    Different,
}

pub fn mount_difference<'a, T: Value>(
    observed: &'a [T],
    expected: &'a [String],
) -> Option<MountDifference<'a>> {
    if expected.is_empty() {
        return Some(MountDifference::NoneExpected);
    }
    for (position, (actual, wanted)) in observed.iter().zip(expected).enumerate() {
        if let Err(folders) = same_mount(actual.as_bytes(), wanted) {
            return Some(MountDifference::Differs {
                position,
                observed: actual.as_bytes(),
                expected: wanted,
                folders,
            });
        }
    }
    let position = observed.len().min(expected.len());
    match (observed.get(position), expected.get(position)) {
        (Some(actual), _) => Some(MountDifference::Extra {
            position,
            observed: actual.as_bytes(),
        }),
        (None, Some(wanted)) => Some(MountDifference::Missing {
            position,
            expected: wanted,
        }),
        (None, None) => None,
    }
}

fn same_mount(observed: &[u8], expected: &str) -> Result<(), Folders> {
    let observed = std::str::from_utf8(observed).map_err(|_| Folders::Unreadable)?;
    match (mount_path(observed), mount_path(expected)) {
        (Some(actual), Some(wanted)) if actual == wanted => return Ok(()),
        (Some(_), Some(_)) => {}
        _ => return Err(Folders::Unreadable),
    }
    match (folder(observed), folder(expected)) {
        (Some(actual), Some(wanted)) if actual == wanted => Ok(()),
        (Some(_), Some(_)) => Err(Folders::Different),
        (actual, wanted) => Err(Folders::Unopened {
            observed: actual.is_some(),
            expected: wanted.is_some(),
        }),
    }
}

fn folder(path: &str) -> Option<(u32, u64)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    };
    let opened = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let read = unsafe { GetFileInformationByHandle(opened.as_raw_handle(), &raw mut info) };
    (read != 0 && info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0).then(|| {
        (
            info.dwVolumeSerialNumber,
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        )
    })
}

fn mount_path(path: &str) -> Option<String> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    let slashes = path.replace('\\', "/");
    let mut normalized = if let Some(unc) = slashes.strip_prefix("//?/UNC/") {
        format!("//{unc}")
    } else {
        slashes.strip_prefix("//?/").unwrap_or(&slashes).to_owned()
    };
    let bytes = normalized.as_bytes();
    let root_len = if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'/'
    {
        3
    } else {
        usize::from(normalized.starts_with('/'))
    };
    normalized.truncate(normalized.trim_end_matches('/').len().max(root_len));
    (!normalized.is_empty()).then_some(normalized)
}

fn read_c_string(memory: &impl Memory, pointer: usize) -> Result<Descriptor, &'static str> {
    let mut text = Descriptor::default();
    for index in 0..=text.bytes.len() {
        let byte = pointer
            .checked_add(index)
            .and_then(|p| read::<u8>(memory, p))
            .ok_or("search path string unreadable")?;
        if byte == 0 {
            text.len = index;
            return Ok(text);
        }
        let Some(target) = text.bytes.get_mut(index) else {
            return Err("search path string exceeds capacity or has no terminator");
        };
        *target = byte;
    }
    Err("search path string exceeds capacity or has no terminator")
}

fn read_string<T: Value>(
    memory: &impl Memory,
    address: usize,
    value: &mut T,
    growing: bool,
) -> Result<(), &'static str> {
    if growing {
        *value = T::read(memory, address, false)?;
        Ok(())
    } else {
        value.read_prepared(memory, address)
    }
}

fn field(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("content address overflow")
}

fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    read::<u64>(memory, address)
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("content pointer unreadable")
}

fn read_strings_inner<D: Value>(
    memory: &impl Memory,
    manager: usize,
    offset: usize,
    output: &mut [D],
    growing: bool,
) -> Outcome {
    let result = (|| {
        let array = Array::read(memory, field(manager, offset)?, STRIDE)
            .ok_or("content string array unreadable or invalid")?;
        let len = (array.count as usize).min(output.len());
        for (index, row) in output[..len].iter_mut().enumerate() {
            let address = array
                .element(index as u32, STRIDE)
                .ok_or("requested content address overflow")?;
            read_string(memory, address, row, growing)
                .map_err(|_| "content string unreadable or exceeds prepared storage")?;
            if row.as_bytes().contains(&0) {
                return Err("content string contains an interior NUL");
            }
        }
        Ok(if len < array.count as usize {
            Outcome::Truncated {
                len,
                unread_rows: array.count - len as u32,
            }
        } else {
            Outcome::Rows { len }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
fn walk_map(
    memory: &impl Memory,
    manager: usize,
    offset: usize,
    output: &mut [Entry],
    active: impl FnMut(usize, &Descriptor) -> Result<bool, &'static str>,
) -> Outcome {
    walk_map_inner(memory, manager, offset, output, false, active)
}

fn walk_map_inner<N: Value, D: Value>(
    memory: &impl Memory,
    manager: usize,
    offset: usize,
    output: &mut [Entry<N, D>],
    growing: bool,
    mut active: impl FnMut(usize, &D) -> Result<bool, &'static str>,
) -> Outcome {
    let result = (|| {
        let map = field(manager, offset)?;
        let head = pointer(memory, field(map, msvc_map::HEAD)?)?;
        let count = read::<u64>(memory, field(map, msvc_map::COUNT)?)
            .ok_or("content map count unreadable")?;
        if head == 0 || read::<u8>(memory, field(head, tree_node::NIL)?) != Some(1) {
            return Err("content map sentinel invalid or node limit exceeded");
        }
        let leftmost = pointer(memory, field(head, tree_node::LEFT)?)?;
        let rightmost = pointer(memory, field(head, tree_node::RIGHT)?)?;
        let mut cursor = pointer(memory, field(head, tree_node::PARENT)?)?;
        if count == 0 {
            return if cursor == head && leftmost == head && rightmost == head {
                Ok(Outcome::Rows { len: 0 })
            } else {
                Err("empty content map has live links")
            };
        }
        if pointer(memory, field(cursor, tree_node::PARENT)?)? != head {
            return Err("content map root parent is not its sentinel");
        }
        let mut steps = 0_u64;
        while cursor != head {
            steps += 1;
            if steps > count {
                return Err("content map child cycle");
            }
            let left = pointer(memory, field(cursor, tree_node::LEFT)?)?;
            if left == head {
                break;
            }
            cursor = left;
        }
        if cursor != leftmost {
            return Err("content map leftmost node mismatch");
        }
        let mut len = 0_usize;
        let mut last = head;
        while cursor != head {
            let node = cursor;
            if node == 0 || read::<u8>(memory, field(node, tree_node::NIL)?) != Some(0) {
                return Err("content map node invalid");
            }
            if len as u64 == count {
                return Err("content map has more nodes than declared");
            }
            for link in [tree_node::LEFT, tree_node::RIGHT] {
                let child = pointer(memory, field(node, link)?)?;
                if child != head
                    && (child == 0 || pointer(memory, field(child, tree_node::PARENT)?)? != node)
                {
                    return Err("content map child parent mismatch");
                }
            }
            if len == output.len() {
                return Ok(Outcome::Truncated {
                    len,
                    unread_rows: u32::try_from(count - len as u64)
                        .map_err(|_| "content map unread count overflow")?,
                });
            }
            let value = field(node, content_node::VALUE)?;
            let row = &mut output[len];
            read_string(
                memory,
                field(value, content_entry::DESCRIPTOR)?,
                &mut row.descriptor,
                growing,
            )
            .map_err(|_| "content descriptor unreadable or exceeds prepared storage")?;
            if row.descriptor.as_bytes().contains(&0) {
                return Err("content descriptor contains an interior NUL");
            }
            read_string(
                memory,
                field(value, content_entry::NAME)?,
                &mut row.name,
                growing,
            )
            .map_err(|_| "content name unreadable or exceeds prepared storage")?;
            row.reported_active = active(value, &row.descriptor)?;
            len += 1;
            last = node;
            cursor = pointer(memory, field(node, tree_node::RIGHT)?)?;
            let mut steps = 0_u64;
            if cursor != head {
                loop {
                    steps += 1;
                    if steps > count {
                        return Err("content map child cycle");
                    }
                    let left = pointer(memory, field(cursor, tree_node::LEFT)?)?;
                    if left == head {
                        break;
                    }
                    cursor = left;
                }
            } else {
                let mut child = node;
                cursor = pointer(memory, field(child, tree_node::PARENT)?)?;
                while cursor != head && pointer(memory, field(cursor, tree_node::RIGHT)?)? == child
                {
                    steps += 1;
                    if steps > count {
                        return Err("content map parent cycle");
                    }
                    child = cursor;
                    cursor = pointer(memory, field(child, tree_node::PARENT)?)?;
                }
            }
        }
        if len as u64 != count || last != rightmost {
            Err("content map count or rightmost node mismatch")
        } else {
            Ok(Outcome::Rows { len })
        }
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn requested_matches(requested: &[u8], descriptor: &[u8]) -> bool {
    let separator = if requested.contains(&b'\\') {
        b'\\'
    } else {
        b'/'
    };
    let mut parts = requested.rsplit(|&c| c == separator);
    let Some(last) = parts.next() else {
        return false;
    };
    let Some(previous) = parts.next() else {
        return false;
    };
    descriptor.len() == previous.len() + 1 + last.len()
        && descriptor.starts_with(previous)
        && descriptor[previous.len()] == b'/'
        && descriptor[previous.len() + 1..] == *last
}

#[cfg(test)]
mod tests;
