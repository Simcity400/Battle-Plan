//! MSVC `std::string`. The derivation is tight, but it is an ABI argument rather

use crate::engine_localisation_provenance::{TargetImageMemory, A729_PREFERRED_IMAGE_BASE};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const DLC_MANAGER_VA_A729: u64 = 0x1_4343_1608;

pub const VFS_SEARCH_PATH_CACHE_VA_A729: u64 = 0x1_435c_af10;

pub const DLC_MANAGER_RVA: u64 = DLC_MANAGER_VA_A729 - A729_PREFERRED_IMAGE_BASE;

pub const VFS_SEARCH_PATH_CACHE_RVA: u64 =
    VFS_SEARCH_PATH_CACHE_VA_A729 - A729_PREFERRED_IMAGE_BASE;

const MANAGER_REQUESTED_DISABLED_DLCS: u64 = 0x50;
const MANAGER_REQUESTED_MODS: u64 = 0x68;
const MANAGER_DISCOVERED_DLC_MAP: u64 = 0xa0;
const MANAGER_DISCOVERED_MOD_MAP: u64 = 0xb0;

const PDX_VECTOR_CAPACITY: u64 = 0x08;
const PDX_VECTOR_SIZE: u64 = 0x0c;

const MAP_HEAD_POINTER: u64 = 0x00;
const MAP_SIZE: u64 = 0x08;

const NODE_LEFT: u64 = 0x00;
const NODE_PARENT: u64 = 0x08;
const NODE_RIGHT: u64 = 0x10;
const NODE_COLOUR: u64 = 0x18;
const NODE_IS_NIL: u64 = 0x19;
const NODE_KEY: u64 = 0x20;
const NODE_VALUE: u64 = 0x40;

const STD_STRING_STRIDE: u64 = 0x20;
const STD_STRING_SIZE_OFFSET: usize = 0x10;
const STD_STRING_CAPACITY_OFFSET: usize = 0x18;
const STD_STRING_SSO_CAPACITY: u64 = 15;

const ENTRY_NAME: u64 = 0x08;
const ENTRY_LOGGED_NAME: u64 = 0x28;
const ENTRY_DESCRIPTOR: u64 = 0x48;
const ENTRY_PATH: u64 = 0x68;
const ENTRY_ARCHIVE: u64 = 0x88;
const ENTRY_DEPENDENCIES: u64 = 0xa8;
const ENTRY_REPLACE_PATHS: u64 = 0xc0;
const ENTRY_BASE_DIRECTORY: u64 = 0x118;
const ENTRY_KIND: u64 = 0x248;
const ENTRY_DLC_ELIGIBLE: u64 = 0x298;

const ENTRY_KIND_DLC: i32 = 1;

const MAX_ENTRIES_PER_MAP: u64 = 4_096;
const MAX_REQUESTED: u32 = 4_096;
const MAX_STRINGS_PER_ENTRY: u32 = 4_096;
const MAX_SEARCH_PATH_ROOTS: usize = 8_192;
const MAX_TEXT_BYTES: usize = 8 * 1024;

const MAX_TOTAL_STRINGS: usize = 65_536;
const MAX_TOTAL_TEXT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct Budget {
    strings: std::cell::Cell<usize>,
    bytes: std::cell::Cell<usize>,
}

impl Budget {
    fn charge(&self, len: usize, label: &str) -> Result<(), String> {
        let strings = self.strings.get() + 1;
        let bytes = self
            .bytes
            .get()
            .checked_add(len)
            .ok_or("engine roster text budget overflow")?;
        if strings > MAX_TOTAL_STRINGS || bytes > MAX_TOTAL_TEXT_BYTES {
            return Err(format!(
                "engine roster exceeded its whole-read text budget at {label}"
            ));
        }
        self.strings.set(strings);
        self.bytes.set(bytes);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EngineContentKind {
    Dlc,
    Mod,
}

impl EngineContentKind {
    fn from_raw(raw: i32) -> Self {
        if raw == ENTRY_KIND_DLC {
            Self::Dlc
        } else {
            Self::Mod
        }
    }

    fn tag(self) -> u8 {
        match self {
            Self::Dlc => 1,
            Self::Mod => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineContentEntry {
    pub kind: EngineContentKind,
    pub map_key: String,
    pub name: String,
    pub descriptor_entry: String,
    pub logged_name: String,
    pub path: Option<String>,
    pub archive: Option<String>,
    pub base_directory: Option<String>,
    pub dependencies: Vec<String>,
    pub replace_paths: Vec<String>,
    pub dlc_eligible: bool,
    pub selected: bool,
    pub mounted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineModsetProvenance {
    pub schema: &'static str,
    pub requested_mods: Vec<String>,
    pub requested_disabled_dlcs: Vec<String>,
    pub discovered: Vec<EngineContentEntry>,
    pub mounted_roots: Vec<String>,
    pub requested_mods_not_discovered: Vec<String>,
    pub selected_not_mounted: Vec<String>,
    pub provenance_sha256: String,
}

impl EngineModsetProvenance {
    pub fn has_disagreement(&self) -> bool {
        !self.requested_mods_not_discovered.is_empty() || !self.selected_not_mounted.is_empty()
    }
}

const PREIMAGE_SCHEMA: &str = "hoi4.engine-modset-provenance.v1";

pub fn read_engine_modset_provenance(
    memory: &dyn TargetImageMemory,
    image_base: u64,
) -> Result<EngineModsetProvenance, String> {
    let first = read_engine_modset_provenance_once(memory, image_base)?;
    let second = read_engine_modset_provenance_once(memory, image_base)?;
    if first != second {
        return Err("engine modset provenance changed between two consecutive reads".to_string());
    }
    Ok(first)
}

pub fn read_engine_modset_provenance_once(
    memory: &dyn TargetImageMemory,
    image_base: u64,
) -> Result<EngineModsetProvenance, String> {
    let manager_slot = image_base
        .checked_add(DLC_MANAGER_RVA)
        .ok_or("DLC manager slot address overflow")?;
    let manager = read_u64(memory, manager_slot, "DLC manager")?;
    if manager == 0 {
        return Err("engine DLC/mod manager is not constructed".to_string());
    }

    let budget = Budget::default();
    let requested_disabled_dlcs = read_string_vector(
        memory,
        field(manager, MANAGER_REQUESTED_DISABLED_DLCS)?,
        MAX_REQUESTED,
        "requested disabled DLC",
        &budget,
    )?;
    let requested_mods = read_string_vector(
        memory,
        field(manager, MANAGER_REQUESTED_MODS)?,
        MAX_REQUESTED,
        "requested mod",
        &budget,
    )?;

    let mounted_roots = read_search_path(memory, image_base, &budget)?;
    let normalized_roots: Vec<String> = mounted_roots.iter().map(|root| normalize(root)).collect();

    let disabled: BTreeSet<&str> = requested_disabled_dlcs.iter().map(String::as_str).collect();
    let enabled: BTreeSet<&str> = requested_mods.iter().map(String::as_str).collect();

    let mut discovered = Vec::new();
    let mut candidates: Vec<Vec<String>> = Vec::new();
    for (map_offset, kind_label) in [
        (MANAGER_DISCOVERED_DLC_MAP, "DLC"),
        (MANAGER_DISCOVERED_MOD_MAP, "mod"),
    ] {
        for (map_key, value) in read_map(memory, field(manager, map_offset)?, kind_label, &budget)?
        {
            let kind = EngineContentKind::from_raw(read_i32(
                memory,
                field(value, ENTRY_KIND)?,
                "content entry kind",
            )?);
            let dlc_eligible =
                read_u8(memory, field(value, ENTRY_DLC_ELIGIBLE)?, "DLC eligibility")? != 0;
            let descriptor_entry =
                read_entry_string(memory, value, ENTRY_DESCRIPTOR, "descriptor entry", &budget)?;
            let path = non_empty(read_entry_string(
                memory,
                value,
                ENTRY_PATH,
                "entry path",
                &budget,
            )?);
            let archive = non_empty(read_entry_string(
                memory,
                value,
                ENTRY_ARCHIVE,
                "entry archive",
                &budget,
            )?);
            let selected = match kind {
                EngineContentKind::Dlc => {
                    dlc_eligible && !disabled.contains(descriptor_entry.as_str())
                }
                EngineContentKind::Mod => enabled.contains(descriptor_entry.as_str()),
            };
            let base_directory = non_empty(read_entry_string(
                memory,
                value,
                ENTRY_BASE_DIRECTORY,
                "entry base directory",
                &budget,
            )?);
            candidates.push(mount_candidates(
                kind,
                base_directory.as_deref(),
                archive.as_deref(),
                path.as_deref(),
            ));
            discovered.push(EngineContentEntry {
                kind,
                map_key,
                name: read_entry_string(memory, value, ENTRY_NAME, "entry name", &budget)?,
                descriptor_entry,
                logged_name: read_entry_string(
                    memory,
                    value,
                    ENTRY_LOGGED_NAME,
                    "logged name",
                    &budget,
                )?,
                path,
                archive,
                base_directory,
                dependencies: read_string_vector(
                    memory,
                    field(value, ENTRY_DEPENDENCIES)?,
                    MAX_STRINGS_PER_ENTRY,
                    "entry dependency",
                    &budget,
                )?,
                replace_paths: read_string_vector(
                    memory,
                    field(value, ENTRY_REPLACE_PATHS)?,
                    MAX_STRINGS_PER_ENTRY,
                    "entry replace path",
                    &budget,
                )?,
                dlc_eligible,
                selected,
                mounted: false,
            });
        }
    }

    resolve_mounted(&mut discovered, &candidates, &normalized_roots);

    discovered.sort_by(|left, right| {
        (left.kind, left.map_key.as_str()).cmp(&(right.kind, right.map_key.as_str()))
    });
    let mut seen = BTreeSet::new();
    for entry in &discovered {
        if !seen.insert((entry.kind, entry.map_key.as_str())) {
            return Err(format!(
                "engine content roster repeats {:?} key {:?}",
                entry.kind, entry.map_key
            ));
        }
    }

    let discovered_mod_descriptors: BTreeSet<&str> = discovered
        .iter()
        .filter(|entry| entry.kind == EngineContentKind::Mod)
        .map(|entry| entry.descriptor_entry.as_str())
        .collect();
    let requested_mods_not_discovered = requested_mods
        .iter()
        .filter(|requested| !discovered_mod_descriptors.contains(requested.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let selected_not_mounted = discovered
        .iter()
        .filter(|entry| entry.selected && !entry.mounted)
        .map(|entry| entry.descriptor_entry.clone())
        .collect::<Vec<_>>();

    let provenance_sha256 = digest(
        &requested_mods,
        &requested_disabled_dlcs,
        &discovered,
        &requested_mods_not_discovered,
        &selected_not_mounted,
    );
    Ok(EngineModsetProvenance {
        schema: PREIMAGE_SCHEMA,
        requested_mods,
        requested_disabled_dlcs,
        discovered,
        mounted_roots,
        requested_mods_not_discovered,
        selected_not_mounted,
        provenance_sha256,
    })
}

fn normalize(path: &str) -> String {
    let lowered = path.to_ascii_lowercase().replace('\\', "/");
    lowered.trim_end_matches('/').to_string()
}

fn mount_candidates(
    kind: EngineContentKind,
    base_directory: Option<&str>,
    archive: Option<&str>,
    path: Option<&str>,
) -> Vec<String> {
    let (relative, raw_allowed) = match (kind, archive, path) {
        (_, None, None) => return Vec::new(),
        (EngineContentKind::Dlc, Some(archive), _) => (archive, false),
        (EngineContentKind::Dlc, None, Some(path)) => (path, false),
        (EngineContentKind::Mod, Some(archive), _) => (archive, true),
        (EngineContentKind::Mod, None, Some(path)) => (path, is_engine_absolute(path)),
    };
    let mut out: Vec<String> = Vec::new();
    for (candidate, allowed) in [
        (
            normalize(&engine_join(base_directory.unwrap_or(""), relative)),
            true,
        ),
        (normalize(relative), raw_allowed),
    ] {
        if allowed && !candidate.is_empty() && !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

fn is_engine_absolute(text: &str) -> bool {
    matches!(
        text.as_bytes(),
        [b'/', ..] | [b'\\', ..] | [_, b':', b'/', ..] | [_, b':', b'\\', ..]
    )
}

fn resolve_mounted(
    discovered: &mut [EngineContentEntry],
    candidates: &[Vec<String>],
    normalized_roots: &[String],
) {
    let roots: BTreeSet<&str> = normalized_roots.iter().map(String::as_str).collect();
    for (entry, candidates) in discovered.iter_mut().zip(candidates) {
        entry.mounted = candidates
            .iter()
            .any(|candidate| roots.contains(candidate.as_str()));
    }
}

fn engine_join(base: &str, relative: &str) -> String {
    if base.is_empty() {
        return relative.to_string();
    }
    if relative.is_empty() {
        return base.to_string();
    }
    if base.ends_with('/') || base.ends_with('\\') {
        format!("{base}{relative}")
    } else {
        format!("{base}/{relative}")
    }
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

fn read_search_path(
    memory: &dyn TargetImageMemory,
    image_base: u64,
    budget: &Budget,
) -> Result<Vec<String>, String> {
    let cache_slot = image_base
        .checked_add(VFS_SEARCH_PATH_CACHE_RVA)
        .ok_or("VFS search path cache address overflow")?;
    let list = read_u64(memory, cache_slot, "VFS search path cache")?;
    if list == 0 {
        return Err("engine VFS search path has not been populated".to_string());
    }
    let mut roots = Vec::new();
    for index in 0..MAX_SEARCH_PATH_ROOTS {
        let slot = list
            .checked_add((index as u64) * 8)
            .ok_or("VFS search path slot address overflow")?;
        let entry = read_u64(memory, slot, "VFS search path entry")?;
        if entry == 0 {
            return Ok(roots);
        }
        roots.push(read_c_string(
            memory,
            entry,
            "VFS search path entry",
            budget,
        )?);
    }
    Err("engine VFS search path is unbounded".to_string())
}

fn read_map(
    memory: &dyn TargetImageMemory,
    container: u64,
    label: &str,
    budget: &Budget,
) -> Result<Vec<(String, u64)>, String> {
    let head = read_u64(memory, field(container, MAP_HEAD_POINTER)?, label)?;
    if head == 0 {
        return Err(format!("engine {label} map has no head node"));
    }
    if !is_nil(memory, head, label)? {
        return Err(format!("engine {label} map head is not a sentinel node"));
    }
    let size = read_u64(memory, field(container, MAP_SIZE)?, label)?;
    if size > MAX_ENTRIES_PER_MAP {
        return Err(format!("engine {label} map size is unbounded"));
    }
    let root = read_u64(memory, field(head, NODE_PARENT)?, label)?;
    if size != 0 && read_u64(memory, field(root, NODE_PARENT)?, label)? != head {
        return Err(format!(
            "engine {label} map root is not linked back to its sentinel"
        ));
    }
    if size == 0 {
        let left = read_u64(memory, field(head, NODE_LEFT)?, label)?;
        let right = read_u64(memory, field(head, NODE_RIGHT)?, label)?;
        if root != head || left != head || right != head {
            return Err(format!(
                "engine {label} map counts no entries but its sentinel is not self-referential"
            ));
        }
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let mut stack = Vec::new();
    let mut cursor = root;
    let mut first_node = 0u64;
    let mut last_node = 0u64;
    loop {
        if cursor == 0 {
            return Err(format!(
                "engine {label} map child link is neither the sentinel nor a live node"
            ));
        }
        if cursor != head && !is_nil(memory, cursor, label)? {
            stack.push(cursor);
            if stack.len() as u64 > size {
                return Err(format!(
                    "engine {label} map traversal exceeded its node count"
                ));
            }
            cursor = read_u64(memory, field(cursor, NODE_LEFT)?, label)?;
            continue;
        }
        let Some(node) = stack.pop() else {
            break;
        };
        if out.len() as u64 >= size {
            return Err(format!(
                "engine {label} map holds more nodes than it counts"
            ));
        }
        if first_node == 0 {
            first_node = node;
        }
        last_node = node;
        for link in [NODE_LEFT, NODE_RIGHT] {
            let child = read_u64(memory, field(node, link)?, label)?;
            if child == head {
                continue;
            }
            if child == 0 || is_nil(memory, child, label)? {
                return Err(format!(
                    "engine {label} map child link is neither the sentinel nor a live node"
                ));
            }
            if read_u64(memory, field(child, NODE_PARENT)?, label)? != node {
                return Err(format!(
                    "engine {label} map child does not point back at its parent"
                ));
            }
        }
        out.push((
            read_std_string(memory, field(node, NODE_KEY)?, label, budget)?,
            field(node, NODE_VALUE)?,
        ));
        cursor = read_u64(memory, field(node, NODE_RIGHT)?, label)?;
    }
    if out.len() as u64 != size {
        return Err(format!(
            "engine {label} map holds {} nodes but counts {size}",
            out.len()
        ));
    }
    if out.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
        return Err(format!(
            "engine {label} map keys are not strictly increasing in order"
        ));
    }
    let leftmost = read_u64(memory, field(head, NODE_LEFT)?, label)?;
    let rightmost = read_u64(memory, field(head, NODE_RIGHT)?, label)?;
    if leftmost != first_node || rightmost != last_node {
        return Err(format!(
            "engine {label} map sentinel does not bracket the nodes it holds"
        ));
    }
    Ok(out)
}

fn is_nil(memory: &dyn TargetImageMemory, node: u64, label: &str) -> Result<bool, String> {
    if read_u8(memory, field(node, NODE_COLOUR)?, label)? > 1 {
        return Err(format!(
            "engine {label} map node colour is not red or black"
        ));
    }
    match read_u8(memory, field(node, NODE_IS_NIL)?, label)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(format!("engine {label} map node nil flag is not a boolean")),
    }
}

fn read_entry_string(
    memory: &dyn TargetImageMemory,
    value: u64,
    offset: u64,
    label: &str,
    budget: &Budget,
) -> Result<String, String> {
    read_std_string(memory, field(value, offset)?, label, budget)
}

fn read_string_vector(
    memory: &dyn TargetImageMemory,
    header: u64,
    bound: u32,
    label: &str,
    budget: &Budget,
) -> Result<Vec<String>, String> {
    let data = read_u64(memory, header, label)?;
    let capacity = read_i32(memory, field(header, PDX_VECTOR_CAPACITY)?, label)?;
    let size = read_i32(memory, field(header, PDX_VECTOR_SIZE)?, label)?;
    if capacity < 0 || size < 0 {
        return Err(format!(
            "engine {label} vector has a negative size or capacity"
        ));
    }
    let (capacity, size) = (capacity as u32, size as u32);
    if size > capacity {
        return Err(format!("engine {label} vector size exceeds its capacity"));
    }
    if size > bound {
        return Err(format!("engine {label} vector size is unbounded"));
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    if data == 0 {
        return Err(format!(
            "engine {label} vector counts entries but holds none"
        ));
    }
    let mut out = Vec::new();
    for index in 0..u64::from(size) {
        let element = data
            .checked_add(index * STD_STRING_STRIDE)
            .ok_or_else(|| format!("engine {label} vector element address overflow"))?;
        out.push(read_std_string(memory, element, label, budget)?);
    }
    Ok(out)
}

fn read_std_string(
    memory: &dyn TargetImageMemory,
    address: u64,
    label: &str,
    budget: &Budget,
) -> Result<String, String> {
    let header = memory
        .read(address, STD_STRING_STRIDE as usize)
        .map_err(|error| format!("read engine {label} string: {error}"))?;
    let size = u64_at(&header, STD_STRING_SIZE_OFFSET)?;
    let capacity = u64_at(&header, STD_STRING_CAPACITY_OFFSET)?;
    if size > capacity {
        return Err(format!("engine {label} string size exceeds its capacity"));
    }
    if size as usize > MAX_TEXT_BYTES {
        return Err(format!("engine {label} string is longer than the bound"));
    }
    let size = size as usize;
    budget.charge(size, label)?;
    let bytes = if capacity > STD_STRING_SSO_CAPACITY {
        let heap = u64_at(&header, 0)?;
        if heap == 0 {
            return Err(format!("engine {label} string claims a null heap buffer"));
        }
        if size == 0 {
            Vec::new()
        } else {
            memory
                .read(heap, size)
                .map_err(|error| format!("read engine {label} string body: {error}"))?
        }
    } else {
        header[..size].to_vec()
    };
    let text =
        String::from_utf8(bytes).map_err(|_| format!("engine {label} string is not UTF-8"))?;
    if text.as_bytes().contains(&0) {
        return Err(format!("engine {label} string contains an embedded NUL"));
    }
    Ok(text)
}

fn read_c_string(
    memory: &dyn TargetImageMemory,
    address: u64,
    label: &str,
    budget: &Budget,
) -> Result<String, String> {
    const CHUNK: u64 = 256;
    const PAGE: u64 = 4096;
    let mut bytes: Vec<u8> = Vec::new();
    let mut cursor = address;
    while bytes.len() < MAX_TEXT_BYTES {
        let to_page_end = PAGE - (cursor % PAGE);
        let remaining = (MAX_TEXT_BYTES - bytes.len()) as u64;
        let take = CHUNK.min(to_page_end).min(remaining) as usize;
        let chunk = memory
            .read(cursor, take)
            .map_err(|error| format!("read engine {label} text: {error}"))?;
        if let Some(end) = chunk.iter().position(|byte| *byte == 0) {
            bytes.extend_from_slice(&chunk[..end]);
            budget.charge(bytes.len(), label)?;
            return String::from_utf8(bytes)
                .map_err(|_| format!("engine {label} text is not UTF-8"));
        }
        bytes.extend_from_slice(&chunk);
        cursor = cursor
            .checked_add(take as u64)
            .ok_or_else(|| format!("engine {label} text address overflow"))?;
    }
    Err(format!(
        "engine {label} text is not NUL-terminated within the bound"
    ))
}

fn digest(
    requested_mods: &[String],
    requested_disabled_dlcs: &[String],
    discovered: &[EngineContentEntry],
    requested_mods_not_discovered: &[String],
    selected_not_mounted: &[String],
) -> String {
    let _ = (requested_mods_not_discovered, selected_not_mounted);
    let mut hasher = Sha256::new();
    write_framed(&mut hasher, PREIMAGE_SCHEMA.as_bytes());
    for list in [requested_mods, requested_disabled_dlcs] {
        write_list(&mut hasher, list);
    }
    hasher.update((discovered.len() as u64).to_le_bytes());
    for entry in discovered {
        hasher.update([entry.kind.tag()]);
        write_framed(&mut hasher, entry.map_key.as_bytes());
        write_framed(&mut hasher, entry.name.as_bytes());
        write_framed(&mut hasher, entry.descriptor_entry.as_bytes());
        write_framed(&mut hasher, entry.logged_name.as_bytes());
        write_list(&mut hasher, &entry.dependencies);
        write_list(&mut hasher, &entry.replace_paths);
        hasher.update([u8::from(entry.dlc_eligible), u8::from(entry.mounted)]);
    }
    format!("{:x}", hasher.finalize())
}

fn write_list(hasher: &mut Sha256, list: &[String]) {
    hasher.update((list.len() as u64).to_le_bytes());
    for item in list {
        write_framed(hasher, item.as_bytes());
    }
}

fn write_framed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn field(base: u64, offset: u64) -> Result<u64, String> {
    base.checked_add(offset)
        .ok_or_else(|| format!("engine field address overflow at +0x{offset:x}"))
}

fn read_u64(memory: &dyn TargetImageMemory, address: u64, label: &str) -> Result<u64, String> {
    let bytes = memory
        .read(address, 8)
        .map_err(|error| format!("read engine {label}: {error}"))?;
    u64_at(&bytes, 0)
}

fn read_u32(memory: &dyn TargetImageMemory, address: u64, label: &str) -> Result<u32, String> {
    let bytes = memory
        .read(address, 4)
        .map_err(|error| format!("read engine {label}: {error}"))?;
    Ok(u32::from_le_bytes(
        bytes[..4].try_into().map_err(|_| "short 32-bit read")?,
    ))
}

fn read_i32(memory: &dyn TargetImageMemory, address: u64, label: &str) -> Result<i32, String> {
    Ok(read_u32(memory, address, label)? as i32)
}

fn read_u8(memory: &dyn TargetImageMemory, address: u64, label: &str) -> Result<u8, String> {
    let bytes = memory
        .read(address, 1)
        .map_err(|error| format!("read engine {label}: {error}"))?;
    bytes
        .first()
        .copied()
        .ok_or_else(|| "short byte read".to_string())
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset.checked_add(8).ok_or("engine read offset overflow")?;
    let slice = bytes
        .get(offset..end)
        .ok_or("engine read produced too few bytes")?;
    Ok(u64::from_le_bytes(
        slice.try_into().map_err(|_| "short 64-bit read")?,
    ))
}

#[cfg(test)]
mod tests;
