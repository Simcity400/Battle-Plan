use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const LOCALISATION_ROOT_VA_A729: u64 = 0x1_435a_10d8;

pub const A729_PREFERRED_IMAGE_BASE: u64 = 0x1_4000_0000;

pub const LOCALISATION_ROOT_RVA: u64 = LOCALISATION_ROOT_VA_A729 - A729_PREFERRED_IMAGE_BASE;

const ROOT_LANGUAGE_STRING_PTR: u64 = 0x00;
const ROOT_TABLE_VECTOR: u64 = 0x08;
const ROOT_TEXT_ARENA_VECTOR: u64 = 0x20;

const TABLE_NAME_STRING: u64 = 0x00;
const TABLE_ENTRY_VECTOR: u64 = 0x38;
const TABLE_SOURCE_VECTOR: u64 = 0x50;

const PDX_VECTOR_CAPACITY: u64 = 0x08;
const PDX_VECTOR_SIZE: u64 = 0x0c;

const ENTRY_STRIDE: u64 = 0x18;
const ENTRY_KEY_OFFSET: usize = 0x08;
const ENTRY_VALUE_OFFSET: usize = 0x0c;
const ENTRY_SOURCE_INDEX: usize = 0x10;

const STD_STRING_STRIDE: u64 = 0x20;
const STD_STRING_SIZE_OFFSET: usize = 0x10;
const STD_STRING_CAPACITY_OFFSET: usize = 0x18;
const STD_STRING_SSO_CAPACITY: u64 = 15;

const MAX_TABLES: u32 = 256;
const MAX_ENTRIES: u32 = 4_000_000;
const MAX_SOURCES: u32 = 200_000;
const MAX_TEXT_BYTES: usize = 64 * 1024;

pub trait TargetImageMemory {
    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineLocalisationKey {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineLocalisationProvenance {
    pub schema: &'static str,
    pub language: String,
    pub key_count: u32,
    pub sources: Vec<String>,
    pub provenance_sha256: String,
}

const PREIMAGE_SCHEMA: &str = "hoi4.d98-engine-localisation-provenance.v1";

pub fn read_engine_localisation_provenance(
    memory: &dyn TargetImageMemory,
    image_base: u64,
    expected_language: &str,
) -> Result<EngineLocalisationProvenance, String> {
    let first = read_engine_localisation_provenance_once(memory, image_base, expected_language)?;
    let second = read_engine_localisation_provenance_once(memory, image_base, expected_language)?;
    if first != second {
        return Err(
            "engine localisation provenance changed between two consecutive reads".to_string(),
        );
    }
    Ok(first)
}

pub fn read_engine_localisation_provenance_once(
    memory: &dyn TargetImageMemory,
    image_base: u64,
    expected_language: &str,
) -> Result<EngineLocalisationProvenance, String> {
    if expected_language.is_empty() {
        return Err("engine localisation provenance requires a launch-bound language".to_string());
    }
    let root_slot = image_base
        .checked_add(LOCALISATION_ROOT_RVA)
        .ok_or("localisation root slot address overflow")?;
    let root = read_u64(memory, root_slot, "localisation root")?;
    if root == 0 {
        return Err("engine localisation system is not initialised".to_string());
    }

    let language_string = read_u64(
        memory,
        field(root, ROOT_LANGUAGE_STRING_PTR)?,
        "active language string pointer",
    )?;
    if language_string == 0 {
        return Err("engine localisation root carries no active language".to_string());
    }
    let language_bytes = read_std_string(memory, language_string, "active language")?;
    let language = String::from_utf8(language_bytes)
        .map_err(|_| "engine active language token is not UTF-8".to_string())?;
    if language != expected_language {
        return Err(format!(
            "engine active language {language:?} differs from the launch-bound language \
             {expected_language:?}"
        ));
    }

    let (table_array, table_count) =
        read_pdx_vector(memory, field(root, ROOT_TABLE_VECTOR)?, "language table")?;
    if table_array == 0 || table_count == 0 {
        return Err("engine localisation holds no language tables".to_string());
    }
    if table_count > MAX_TABLES {
        return Err("engine localisation language table count is unbounded".to_string());
    }
    let (arena, arena_len) =
        read_pdx_vector(memory, field(root, ROOT_TEXT_ARENA_VECTOR)?, "text arena")?;
    if arena == 0 || arena_len == 0 {
        return Err("engine localisation holds no text arena".to_string());
    }

    let table = select_language_table(memory, table_array, table_count, &language)?;
    let entries = read_table_entries(memory, table, arena, arena_len)?;

    let mut sources = BTreeSet::new();
    for entry in &entries {
        if let Some(source) = entry.source.as_ref() {
            sources.insert(source.clone());
        }
    }

    let key_count = u32::try_from(entries.len())
        .map_err(|_| "engine localisation key count does not fit u32".to_string())?;
    Ok(EngineLocalisationProvenance {
        schema: PREIMAGE_SCHEMA,
        provenance_sha256: digest_entries(&language, &entries),
        language,
        key_count,
        sources: sources.into_iter().collect(),
    })
}

fn select_language_table(
    memory: &dyn TargetImageMemory,
    table_array: u64,
    table_count: u32,
    language: &str,
) -> Result<u64, String> {
    let mut found = None;
    for index in 0..u64::from(table_count) {
        let slot = table_array
            .checked_add(index * 8)
            .ok_or("language table slot address overflow")?;
        let table = read_u64(memory, slot, "language table")?;
        if table == 0 {
            return Err("engine localisation language table slot is null".to_string());
        }
        let name = read_std_string(
            memory,
            field(table, TABLE_NAME_STRING)?,
            "language table name",
        )?;
        if name == language.as_bytes() {
            if found.is_some() {
                return Err(format!(
                    "engine localisation holds more than one table for language {language:?}"
                ));
            }
            found = Some(table);
        }
    }
    found.ok_or_else(|| {
        format!("engine localisation holds no table for the active language {language:?}")
    })
}

fn read_table_entries(
    memory: &dyn TargetImageMemory,
    table: u64,
    arena: u64,
    arena_len: u32,
) -> Result<Vec<EngineLocalisationKey>, String> {
    let (entry_array, entry_count) =
        read_pdx_vector(memory, field(table, TABLE_ENTRY_VECTOR)?, "entry")?;
    if entry_array == 0 || entry_count == 0 {
        return Err("engine localisation table holds no entries".to_string());
    }
    if entry_count > MAX_ENTRIES {
        return Err("engine localisation entry count is unbounded".to_string());
    }
    let (source_array, source_count) =
        read_pdx_vector(memory, field(table, TABLE_SOURCE_VECTOR)?, "source")?;
    if source_count > MAX_SOURCES {
        return Err("engine localisation source count is unbounded".to_string());
    }
    if source_count > 0 && source_array == 0 {
        return Err("engine localisation source array is null with a non-zero count".to_string());
    }

    let entry_bytes_len = usize::try_from(u64::from(entry_count) * ENTRY_STRIDE)
        .map_err(|_| "engine localisation entry array does not fit usize".to_string())?;
    let entry_bytes = memory.read(entry_array, entry_bytes_len)?;

    let sources = read_sources(memory, source_array, source_count)?;

    let mut entries = Vec::new();
    entries
        .try_reserve_exact(entry_count as usize)
        .map_err(|error| format!("reserve engine localisation entries: {error}"))?;
    for index in 0..entry_count as usize {
        let base = index * ENTRY_STRIDE as usize;
        let key_offset = i32_at(&entry_bytes, base + ENTRY_KEY_OFFSET)?;
        let value_offset = i32_at(&entry_bytes, base + ENTRY_VALUE_OFFSET)?;
        let source_index = i32_at(&entry_bytes, base + ENTRY_SOURCE_INDEX)?;

        let key = read_arena_text(memory, arena, arena_len, key_offset, "key")?;
        if key.is_empty() {
            return Err("engine localisation entry carries an empty key".to_string());
        }
        let value = read_arena_text(memory, arena, arena_len, value_offset, "value")?;
        let source = if source_index == -1 {
            None
        } else {
            let slot = usize::try_from(source_index)
                .map_err(|_| "engine localisation source index is negative".to_string())?;
            Some(
                sources
                    .get(slot)
                    .ok_or("engine localisation source index is out of range")?
                    .clone(),
            )
        };
        entries.push(EngineLocalisationKey { key, value, source });
    }

    entries.sort_by(|left, right| left.key.cmp(&right.key));
    for pair in entries.windows(2) {
        if pair[0].key == pair[1].key {
            return Err(
                "engine localisation table holds a duplicate key; the table is mid-load"
                    .to_string(),
            );
        }
    }
    Ok(entries)
}

fn read_sources(
    memory: &dyn TargetImageMemory,
    source_array: u64,
    source_count: u32,
) -> Result<Vec<String>, String> {
    let mut sources = Vec::new();
    sources
        .try_reserve_exact(source_count as usize)
        .map_err(|error| format!("reserve engine localisation sources: {error}"))?;
    for index in 0..u64::from(source_count) {
        let address = source_array
            .checked_add(index * STD_STRING_STRIDE)
            .ok_or("engine localisation source slot address overflow")?;
        let bytes = read_std_string(memory, address, "source name")?;
        sources.push(
            String::from_utf8(bytes)
                .map_err(|_| "engine localisation source name is not UTF-8".to_string())?,
        );
    }
    Ok(sources)
}

fn read_arena_text(
    memory: &dyn TargetImageMemory,
    arena: u64,
    arena_len: u32,
    offset: i32,
    label: &str,
) -> Result<Vec<u8>, String> {
    if offset < 0 {
        return Err(format!("engine localisation {label} offset is negative"));
    }
    let offset = u32::try_from(offset)
        .map_err(|_| format!("engine localisation {label} offset is negative"))?;
    if offset >= arena_len {
        return Err(format!(
            "engine localisation {label} offset is out of range"
        ));
    }
    let limit = usize::try_from(arena_len - offset)
        .map_err(|_| format!("engine localisation {label} arena span does not fit usize"))?;
    let address = arena
        .checked_add(u64::from(offset))
        .ok_or_else(|| format!("engine localisation {label} address overflow"))?;
    read_c_string(memory, address, limit, label)
}

fn read_c_string(
    memory: &dyn TargetImageMemory,
    address: u64,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    const CHUNK: u64 = 256;
    const PAGE: u64 = 4096;
    let mut out: Vec<u8> = Vec::new();
    let mut cursor = address;
    loop {
        let remaining = MAX_TEXT_BYTES
            .min(limit)
            .checked_sub(out.len())
            .filter(|remaining| *remaining > 0)
            .ok_or_else(|| format!("engine localisation {label} is unterminated"))?;
        let to_page_end = PAGE - (cursor % PAGE);
        let length = usize::try_from(CHUNK.min(to_page_end))
            .map_err(|_| format!("engine localisation {label} chunk does not fit usize"))?
            .min(remaining);
        let chunk = memory.read(cursor, length)?;
        if let Some(position) = chunk.iter().position(|byte| *byte == 0) {
            out.extend_from_slice(&chunk[..position]);
            return Ok(out);
        }
        out.extend_from_slice(&chunk);
        cursor = cursor
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| format!("engine localisation {label} address overflow"))?;
    }
}

fn read_std_string(
    memory: &dyn TargetImageMemory,
    address: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    let header = memory.read(address, STD_STRING_STRIDE as usize)?;
    let size = u64_at(&header, STD_STRING_SIZE_OFFSET)?;
    let capacity = u64_at(&header, STD_STRING_CAPACITY_OFFSET)?;
    if size > capacity {
        return Err(format!(
            "engine localisation {label} string size exceeds capacity"
        ));
    }
    let size = usize::try_from(size)
        .map_err(|_| format!("engine localisation {label} string size does not fit usize"))?;
    if size > MAX_TEXT_BYTES {
        return Err(format!("engine localisation {label} string is unbounded"));
    }
    if capacity <= STD_STRING_SSO_CAPACITY {
        return Ok(header[..size].to_vec());
    }
    let heap = u64_at(&header, 0)?;
    if heap == 0 {
        return Err(format!(
            "engine localisation {label} string has a null buffer"
        ));
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    memory.read(heap, size)
}

fn digest_entries(language: &str, entries: &[EngineLocalisationKey]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PREIMAGE_SCHEMA.as_bytes());
    hasher.update([0u8]);
    write_framed(&mut hasher, language.as_bytes());
    hasher.update(
        u64::try_from(entries.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for entry in entries {
        write_framed(&mut hasher, &entry.key);
        write_framed(&mut hasher, &entry.value);
        match entry.source.as_ref() {
            Some(source) => {
                hasher.update([1u8]);
                write_framed(&mut hasher, source.as_bytes());
            }
            None => hasher.update([0u8]),
        }
    }
    format!("{:x}", hasher.finalize())
}

fn write_framed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(bytes);
}

fn field(base: u64, offset: u64) -> Result<u64, String> {
    base.checked_add(offset)
        .ok_or_else(|| "engine localisation field address overflow".to_string())
}

fn read_pdx_vector(
    memory: &dyn TargetImageMemory,
    vector: u64,
    label: &str,
) -> Result<(u64, u32), String> {
    let data = read_u64(memory, vector, label)?;
    let capacity = read_u32(
        memory,
        field(vector, PDX_VECTOR_CAPACITY)?,
        &format!("{label} capacity"),
    )?;
    let size = read_u32(
        memory,
        field(vector, PDX_VECTOR_SIZE)?,
        &format!("{label} count"),
    )?;
    if size > capacity {
        return Err(format!(
            "engine localisation {label} vector size exceeds its capacity"
        ));
    }
    if data == 0 && capacity != 0 {
        return Err(format!(
            "engine localisation {label} vector has capacity without storage"
        ));
    }
    Ok((data, size))
}

fn read_u64(memory: &dyn TargetImageMemory, address: u64, label: &str) -> Result<u64, String> {
    let bytes = memory.read(address, 8)?;
    u64_at(&bytes, 0).map_err(|_| format!("engine localisation {label} read short"))
}

fn read_u32(memory: &dyn TargetImageMemory, address: u64, label: &str) -> Result<u32, String> {
    let bytes = memory.read(address, 4)?;
    let raw = i32_at(&bytes, 0).map_err(|_| format!("engine localisation {label} read short"))?;
    u32::try_from(raw).map_err(|_| format!("engine localisation {label} is negative"))
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset
        .checked_add(8)
        .ok_or("engine localisation scalar offset overflow")?;
    let slice = bytes
        .get(offset..end)
        .ok_or("engine localisation scalar read is short")?;
    Ok(u64::from_le_bytes(slice.try_into().unwrap()))
}

fn i32_at(bytes: &[u8], offset: usize) -> Result<i32, String> {
    let end = offset
        .checked_add(4)
        .ok_or("engine localisation scalar offset overflow")?;
    let slice = bytes
        .get(offset..end)
        .ok_or("engine localisation scalar read is short")?;
    Ok(i32::from_le_bytes(slice.try_into().unwrap()))
}

#[cfg(test)]
mod tests;
