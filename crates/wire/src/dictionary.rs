//! Immutable H4RD shards. Pages carry a count, u64 offsets and exact entry bytes.
//! Dictionary identities belong to the caller; shard identities hash these bytes.
use crate::errors::{Result, WireError, align8, checked_size, err, usize_count};
use crate::model::{DictionaryKind, DictionaryRef};
use crate::reader::{u16_at, u64_at};

pub const DICTIONARY_VERSION: u16 = 1;
pub const DICTIONARY_HEADER_BYTES: usize = 32;

#[derive(Clone, Debug)]
pub struct Page {
    pub entries: Vec<Vec<u8>>,
}
#[derive(Clone, Debug)]
pub struct Shard {
    pub bytes: Vec<u8>,
    pub identity: [u8; 32],
}
#[derive(Clone, Debug)]
pub struct ShardPage<'a> {
    pub first_entry: u64,
    pub entry_count: u64,
    pub offsets: &'a [u8],
    pub data: &'a [u8],
}
#[derive(Clone, Debug)]
pub struct ValidatedShard<'a> {
    pub identity: [u8; 32],
    pub kind: DictionaryKind,
    pub first_entry: u64,
    pub entry_count: u64,
    pub pages: Vec<ShardPage<'a>>,
    pub bytes: &'a [u8],
}
#[derive(Clone, Debug)]
pub struct ByteExtent<'a> {
    pub shard_identity: [u8; 32],
    pub canonical_entry: u64,
    pub first: u64,
    pub end: u64,
    pub total: u64,
    pub final_extent: bool,
    pub bytes: &'a [u8],
}

fn grow(out: &mut Vec<u8>, data: &[u8]) -> Result<()> {
    out.len()
        .checked_add(data.len())
        .ok_or_else(|| WireError::new("size-overflow", "dictionary size overflow"))?;
    out.try_reserve(data.len())
        .map_err(|_| WireError::new("size-overflow", "dictionary allocation failed"))?;
    out.extend_from_slice(data);
    Ok(())
}

/// Caller-chosen pages impose no lifetime inventory or entry-size limit.
/// Even a single oversize entry can be fetched through byte_extent.
pub fn write_shard(kind: DictionaryKind, first_entry: u64, pages: &[Page]) -> Result<Shard> {
    let page_count = pages.len() as u64;
    let entries = pages.iter().try_fold(0u64, |count, p| {
        count
            .checked_add(p.entries.len() as u64)
            .ok_or_else(|| WireError::new("dictionary-overflow", "shard entry count overflow"))
    })?;
    if entries > 0 && first_entry.checked_add(entries - 1).is_none() {
        return err("dictionary-overflow", "canonical entry index overflow");
    }
    let table = checked_size(page_count, 24)?;
    let header_end = DICTIONARY_HEADER_BYTES
        .checked_add(table)
        .ok_or_else(|| WireError::new("size-overflow", "dictionary directory overflow"))?;
    let mut out = vec![0; header_end];
    out[..4].copy_from_slice(b"H4RD");
    out[4..6].copy_from_slice(&DICTIONARY_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(kind as u16).to_le_bytes());
    out[8..16].copy_from_slice(&first_entry.to_le_bytes());
    out[16..24].copy_from_slice(&entries.to_le_bytes());
    out[24..32].copy_from_slice(&page_count.to_le_bytes());
    let mut position = first_entry;
    for (i, page) in pages.iter().enumerate() {
        if page.entries.is_empty() {
            return err("dictionary-page", "empty pages are omitted");
        }
        let count = page.entries.len() as u64;
        let offset_count = count
            .checked_add(1)
            .ok_or_else(|| WireError::new("size-overflow", "dictionary offset count overflow"))?;
        let offset_bytes = checked_size(offset_count, 8)?;
        let mut body = Vec::new();
        body.extend_from_slice(&count.to_le_bytes());
        body.resize(
            8usize
                .checked_add(offset_bytes)
                .ok_or_else(|| WireError::new("size-overflow", "dictionary page overflow"))?,
            0,
        );
        let mut cursor = 0u64;
        for (entry_index, entry) in page.entries.iter().enumerate() {
            if matches!(
                kind,
                DictionaryKind::Text
                    | DictionaryKind::Tag
                    | DictionaryKind::Refusal
                    | DictionaryKind::ImageKey
                    | DictionaryKind::DiagnosticCode
            ) {
                std::str::from_utf8(entry).map_err(|_| {
                    WireError::new("invalid-utf8", "dictionary text entry is not UTF-8")
                })?;
            }
            let slot = 8 + entry_index * 8;
            body[slot..slot + 8].copy_from_slice(&cursor.to_le_bytes());
            cursor = cursor.checked_add(entry.len() as u64).ok_or_else(|| {
                WireError::new("size-overflow", "dictionary entry bytes overflow")
            })?;
            grow(&mut body, entry)?;
        }
        let final_slot = 8 + page.entries.len() * 8;
        body[final_slot..final_slot + 8].copy_from_slice(&cursor.to_le_bytes());
        let aligned = align8(out.len())?;
        out.resize(aligned, 0);
        let offset = out.len() as u64;
        let length = body.len() as u64;
        let slot = DICTIONARY_HEADER_BYTES + i * 24;
        out[slot..slot + 8].copy_from_slice(&position.to_le_bytes());
        out[slot + 8..slot + 16].copy_from_slice(&offset.to_le_bytes());
        out[slot + 16..slot + 24].copy_from_slice(&length.to_le_bytes());
        grow(&mut out, &body)?;
        // The exclusive end of a final range can be one above u64::MAX. We
        // need it only if another page follows, never for a canonical index.
        if i + 1 < pages.len() {
            position = position.checked_add(count).ok_or_else(|| {
                WireError::new("dictionary-overflow", "page first-entry overflow")
            })?;
        }
    }
    let identity = *blake3::hash(&out).as_bytes();
    read_shard(&out, identity)?;
    Ok(Shard {
        bytes: out,
        identity,
    })
}

pub fn read_page(bytes: &[u8], first_entry: u64, kind: DictionaryKind) -> Result<ShardPage<'_>> {
    if bytes.len() < 8 {
        return err("dictionary-page", "page lacks entry count");
    }
    let entry_count = u64_at(bytes, 0);
    if entry_count == 0 {
        return err("dictionary-page", "empty page");
    }
    if first_entry.checked_add(entry_count - 1).is_none() {
        return err("dictionary-overflow", "page canonical index overflow");
    }
    let offsets_length = checked_size(
        entry_count
            .checked_add(1)
            .ok_or_else(|| WireError::new("size-overflow", "dictionary offsets overflow"))?,
        8,
    )?;
    let data_start = 8usize
        .checked_add(offsets_length)
        .ok_or_else(|| WireError::new("size-overflow", "dictionary page offset overflow"))?;
    if data_start > bytes.len() {
        return err("dictionary-page", "offset table truncated");
    }
    let offsets = &bytes[8..data_start];
    let data = &bytes[data_start..];
    let mut prior = 0;
    for (i, raw) in offsets.chunks_exact(8).enumerate() {
        let offset = u64_at(raw, 0);
        if (i == 0 && offset != 0) || offset < prior || offset > data.len() as u64 {
            return err("dictionary-page", "invalid dictionary offsets");
        }
        prior = offset;
    }
    if prior != data.len() as u64 {
        return err(
            "dictionary-page",
            "last dictionary offset differs from data length",
        );
    }
    let page = ShardPage {
        first_entry,
        entry_count,
        offsets,
        data,
    };
    if kind != DictionaryKind::Plain {
        for i in 0..entry_count {
            std::str::from_utf8(page.entry(i)?).map_err(|_| {
                WireError::new("invalid-utf8", "dictionary text entry is not UTF-8")
            })?;
        }
    }
    Ok(page)
}

pub fn read_shard(bytes: &[u8], expected_identity: [u8; 32]) -> Result<ValidatedShard<'_>> {
    if bytes.len() < DICTIONARY_HEADER_BYTES || &bytes[..4] != b"H4RD" {
        return err("invalid-dictionary", "expected H4RD header");
    }
    if u16_at(bytes, 4) != DICTIONARY_VERSION {
        return err("dictionary-version", "unsupported shard version");
    }
    let identity = *blake3::hash(bytes).as_bytes();
    if identity != expected_identity {
        return err("dictionary-identity", "shard content differs from address");
    }
    let kind = DictionaryKind::try_from(u32::from(u16_at(bytes, 6)))
        .map_err(|_| WireError::new("dictionary-kind", "unknown shard kind"))?;
    let first_entry = u64_at(bytes, 8);
    let entry_count = u64_at(bytes, 16);
    let page_count = u64_at(bytes, 24);
    if entry_count > 0 && first_entry.checked_add(entry_count - 1).is_none() {
        return err("dictionary-overflow", "shard canonical entry overflow");
    }
    let directory_end = DICTIONARY_HEADER_BYTES
        .checked_add(checked_size(page_count, 24)?)
        .ok_or_else(|| WireError::new("size-overflow", "shard directory overflow"))?;
    if directory_end > bytes.len() {
        return err("dictionary-page", "page directory truncated");
    }
    let mut pages = Vec::new();
    let mut prior_end = directory_end;
    let mut emitted = 0u64;
    for raw in bytes[DICTIONARY_HEADER_BYTES..directory_end].chunks_exact(24) {
        let page_first = u64_at(raw, 0);
        let offset = u64_at(raw, 8);
        let length = u64_at(raw, 16);
        if first_entry.checked_add(emitted) != Some(page_first) {
            return err(
                "dictionary-page",
                "page entries must be contiguous in directory order",
            );
        }
        if !offset.is_multiple_of(8) {
            return err("dictionary-page", "page is not eight-byte aligned");
        }
        let start = usize_count(offset)?;
        let end = usize_count(
            offset
                .checked_add(length)
                .ok_or_else(|| WireError::new("dictionary-page", "page range overflow"))?,
        )?;
        if start < prior_end || end > bytes.len() {
            return err("dictionary-page", "page overlaps or leaves received bytes");
        }
        if bytes[prior_end..start].iter().any(|&v| v != 0) {
            return err(
                "nonzero-padding",
                "dictionary page gap contains nonzero bytes",
            );
        }
        let page = read_page(&bytes[start..end], page_first, kind)?;
        emitted = emitted
            .checked_add(page.entry_count)
            .ok_or_else(|| WireError::new("dictionary-overflow", "page counts overflow"))?;
        prior_end = end;
        pages.push(page);
    }
    if emitted != entry_count || bytes[prior_end..].iter().any(|&v| v != 0) {
        return err(
            "dictionary-page",
            "shard entry count or trailing padding mismatch",
        );
    }
    Ok(ValidatedShard {
        identity,
        kind,
        first_entry,
        entry_count,
        pages,
        bytes,
    })
}
impl ShardPage<'_> {
    pub fn entry(&self, local: u64) -> Result<&[u8]> {
        if local >= self.entry_count {
            return err("dictionary-code", "entry outside page");
        }
        let p = checked_size(local, 8)?;
        Ok(&self.data
            [usize_count(u64_at(self.offsets, p))?..usize_count(u64_at(self.offsets, p + 8))?])
    }
}
impl ValidatedShard<'_> {
    pub fn check_reference(&self, reference: &DictionaryRef) -> Result<()> {
        if reference.shard_identity != self.identity {
            return err("dictionary-identity", "reply reference names another shard");
        }
        if reference.kind != self.kind
            || reference.first_entry != self.first_entry
            || reference.entry_count != self.entry_count
            || reference.flags != 0
        {
            return err(
                "dictionary-reference",
                "reference metadata differs from shard",
            );
        }
        Ok(())
    }
    pub fn entry(&self, code: u64) -> Result<&[u8]> {
        if code >= self.entry_count {
            return err("dictionary-code", "entry outside shard");
        }
        let canonical = self
            .first_entry
            .checked_add(code)
            .ok_or_else(|| WireError::new("dictionary-overflow", "canonical entry overflow"))?;
        let i = self.pages.partition_point(|p| p.first_entry <= canonical);
        let page = self
            .pages
            .get(i.saturating_sub(1))
            .ok_or_else(|| WireError::new("dictionary-page", "entry lacks page"))?;
        page.entry(canonical - page.first_entry)
    }
    pub fn byte_extent(&self, code: u64, first: u64, end: u64) -> Result<ByteExtent<'_>> {
        let entry = self.entry(code)?;
        let total = entry.len() as u64;
        if first > end || end > total {
            return err("dictionary-extents", "byte extent outside entry");
        }
        Ok(ByteExtent {
            shard_identity: self.identity,
            canonical_entry: self
                .first_entry
                .checked_add(code)
                .ok_or_else(|| WireError::new("dictionary-overflow", "canonical entry overflow"))?,
            first,
            end,
            total,
            final_extent: end == total,
            bytes: &entry[usize_count(first)?..usize_count(end)?],
        })
    }
}

#[derive(Clone, Debug)]
pub struct DictionaryFixture {
    pub name: String,
    pub bytes: Vec<u8>,
    pub identity: [u8; 32],
    pub expected_error: Option<&'static str>,
    pub expected_entries: Vec<Vec<u8>>,
    pub byte_extents: Vec<(u64, u64, u64)>,
}

/// Synthetic shard bytes exported alongside the Rust-written reply corpus.
pub fn synthetic_fixtures() -> Result<Vec<DictionaryFixture>> {
    fn valid(
        name: &str,
        kind: DictionaryKind,
        first: u64,
        pages: Vec<Page>,
        extents: Vec<(u64, u64, u64)>,
    ) -> Result<DictionaryFixture> {
        let shard = write_shard(kind, first, &pages)?;
        let expected_entries = pages.into_iter().flat_map(|p| p.entries).collect();
        Ok(DictionaryFixture {
            name: name.into(),
            bytes: shard.bytes,
            identity: shard.identity,
            expected_error: None,
            expected_entries,
            byte_extents: extents,
        })
    }
    fn change(
        out: &mut Vec<DictionaryFixture>,
        base: &DictionaryFixture,
        name: &str,
        error: &'static str,
        change: impl FnOnce(&mut Vec<u8>),
    ) {
        let mut f = base.clone();
        f.name = name.into();
        f.expected_error = Some(error);
        change(&mut f.bytes);
        f.identity = *blake3::hash(&f.bytes).as_bytes();
        out.push(f);
    }
    fn put64(b: &mut [u8], at: usize, value: u64) {
        b[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    let mut out = vec![
        valid(
            "dictionary-wide-canonical-bytes",
            DictionaryKind::Plain,
            u64::from(u32::MAX) + 20,
            vec![
                Page {
                    entries: vec![vec![0xff, 0, 0x80], vec![]],
                },
                Page {
                    entries: vec![b"synthetic".to_vec()],
                },
            ],
            vec![(0, 1, 3)],
        )?,
        valid(
            "dictionary-text-empty-utf8",
            DictionaryKind::Text,
            100,
            vec![Page {
                entries: vec![vec![], "é∅".as_bytes().to_vec()],
            }],
            vec![],
        )?,
        valid(
            "dictionary-refusal-above-255",
            DictionaryKind::Refusal,
            1 << 40,
            vec![Page {
                entries: (0..301)
                    .map(|i| format!("synthetic-reason-{i}").into_bytes())
                    .collect(),
            }],
            vec![],
        )?,
        valid(
            "dictionary-entry-byte-extents",
            DictionaryKind::Plain,
            0,
            vec![Page {
                entries: vec![(0..8193).map(|i| (i % 256) as u8).collect()],
            }],
            vec![(0, 0, 4096), (0, 4095, 4098), (0, 4096, 8193)],
        )?,
        valid(
            "dictionary-final-u64-canonical-entry",
            DictionaryKind::Plain,
            u64::MAX,
            vec![Page {
                entries: vec![vec![7]],
            }],
            vec![],
        )?,
        valid(
            "dictionary-error-stale-generation",
            DictionaryKind::DiagnosticCode,
            0,
            vec![Page {
                entries: vec![b"stale-generation".to_vec()],
            }],
            vec![],
        )?,
    ];
    let base = out[0].clone();
    change(
        &mut out,
        &base,
        "dictionary-bad-magic",
        "invalid-dictionary",
        |b| b[0] = 0,
    );
    change(
        &mut out,
        &base,
        "dictionary-bad-version",
        "dictionary-version",
        |b| b[4] = 2,
    );
    change(
        &mut out,
        &base,
        "dictionary-bad-kind",
        "dictionary-kind",
        |b| b[6] = 99,
    );
    let mut bad_identity = base.clone();
    bad_identity.name = "dictionary-bad-identity".into();
    bad_identity.expected_error = Some("dictionary-identity");
    bad_identity.identity = [0; 32];
    out.push(bad_identity);
    change(
        &mut out,
        &base,
        "dictionary-bad-page-alignment",
        "dictionary-page",
        |b| {
            let offset = u64_at(b, 40);
            put64(b, 40, offset + 1)
        },
    );
    change(
        &mut out,
        &base,
        "dictionary-bad-page-bounds",
        "dictionary-page",
        |b| put64(b, 48, u64::MAX),
    );
    change(
        &mut out,
        &base,
        "dictionary-bad-page-contiguity",
        "dictionary-page",
        |b| put64(b, 56, 0),
    );
    change(
        &mut out,
        &base,
        "dictionary-bad-entry-count",
        "dictionary-page",
        |b| put64(b, 16, 4),
    );
    change(
        &mut out,
        &base,
        "dictionary-bad-canonical-overflow",
        "dictionary-overflow",
        |b| put64(b, 8, u64::MAX),
    );
    change(
        &mut out,
        &base,
        "dictionary-bad-offset-order",
        "dictionary-page",
        |b| {
            let page = u64_at(b, 40) as usize;
            put64(b, page + 16, 4)
        },
    );
    let text = out[1].clone();
    change(
        &mut out,
        &text,
        "dictionary-bad-text-utf8",
        "invalid-utf8",
        |b| {
            let page = u64_at(b, 40) as usize;
            let count = u64_at(b, page) as usize;
            b[page + 8 + (count + 1) * 8] = 0xff
        },
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wide_canonical_entries_and_byte_extents() {
        let shard = write_shard(
            DictionaryKind::Plain,
            u64::from(u32::MAX) + 20,
            &[
                Page {
                    entries: vec![vec![0xff, 0, 2], vec![]],
                },
                Page {
                    entries: vec![vec![3, 4]],
                },
            ],
        )
        .unwrap();
        let decoded = read_shard(&shard.bytes, shard.identity).unwrap();
        assert_eq!(decoded.entry(1).unwrap(), &[] as &[u8]);
        let part = decoded.byte_extent(0, 1, 3).unwrap();
        assert_eq!(part.bytes, &[0, 2]);
        assert_eq!(part.canonical_entry, u64::from(u32::MAX) + 20);
        assert!(part.final_extent);
        assert_eq!(
            read_shard(&shard.bytes, [0; 32]).unwrap_err().name,
            "dictionary-identity"
        );
    }
    #[test]
    fn final_canonical_entry_at_u64_max_is_valid() {
        let shard = write_shard(
            DictionaryKind::Plain,
            u64::MAX,
            &[Page {
                entries: vec![vec![1]],
            }],
        )
        .unwrap();
        assert_eq!(
            read_shard(&shard.bytes, shard.identity)
                .unwrap()
                .entry(0)
                .unwrap(),
            &[1]
        );
        assert_eq!(
            write_shard(
                DictionaryKind::Plain,
                u64::MAX,
                &[Page {
                    entries: vec![vec![], vec![]]
                }]
            )
            .unwrap_err()
            .name,
            "dictionary-overflow"
        );
    }
    #[test]
    fn shared_shard_fixture_corpus() {
        for f in synthetic_fixtures().unwrap() {
            match f.expected_error {
                Some(expected) => assert_eq!(
                    read_shard(&f.bytes, f.identity).unwrap_err().name,
                    expected,
                    "{}",
                    f.name
                ),
                None => {
                    let decoded = read_shard(&f.bytes, f.identity).unwrap();
                    for (code, entry) in f.expected_entries.iter().enumerate() {
                        assert_eq!(
                            decoded.entry(code as u64).unwrap(),
                            entry,
                            "{} entry {code}",
                            f.name
                        );
                    }
                    for (code, first, end) in f.byte_extents {
                        assert_eq!(
                            decoded.byte_extent(code, first, end).unwrap().bytes,
                            &f.expected_entries[code as usize][first as usize..end as usize],
                            "{} byte extent",
                            f.name
                        );
                    }
                }
            }
        }
    }
}
