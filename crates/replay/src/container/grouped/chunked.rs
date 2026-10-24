use super::*;

pub(super) fn slots(index: &BlockIndex) -> io::Result<usize> {
    let span = i64::from(index.last_hour) - i64::from(index.first_hour);
    usize::try_from(span)
        .map(|span| span.div_ceil(LATER_CHUNK_HOURS))
        .map_err(|_| invalid_static("invalid chunk hour range"))
}

pub(super) fn columns(raw: &[u8]) -> io::Result<Vec<&[u8]>> {
    let mut input = Decoder::new(raw);
    let count = varint_usize(&mut input)?;
    if !(FIELDS..=FIELDS + 65).contains(&count) {
        return Err(invalid_static("chunk has the wrong column count"));
    }
    let lengths = (0..count)
        .map(|_| varint_usize(&mut input))
        .collect::<io::Result<Vec<_>>>()?;
    let columns = lengths
        .into_iter()
        .map(|length| input.take(length))
        .collect::<io::Result<Vec<_>>>()?;
    input.finish()?;
    Ok(columns)
}

pub(super) fn frame(columns: &[impl AsRef<[u8]>]) -> io::Result<Vec<u8>> {
    let mut length = crate::rows::varint_bytes(columns.len() as u128);
    for column in columns {
        let bytes = column.as_ref().len();
        length = engine_allocation::add(length, crate::rows::varint_bytes(bytes as u128))?;
        length = engine_allocation::add(length, bytes)?;
    }
    let mut raw = engine_allocation::vec(length)?;
    put_varint(&mut raw, columns.len() as u128);
    for column in columns {
        put_varint(&mut raw, column.as_ref().len() as u128);
    }
    for column in columns {
        raw.extend_from_slice(column.as_ref());
    }
    Ok(raw)
}

pub(super) fn split(id: usize, later: &Later) -> io::Result<Vec<(usize, usize, Vec<u8>)>> {
    let mut previous = vec![0; later.columns.0.len()];
    let mut sections = Vec::new();
    for end in &later.cuts {
        let mut columns = Vec::new();
        for ((column, &start), &end) in later.columns.0.iter().zip(&previous).zip(end) {
            columns.push(
                column
                    .get(start..end)
                    .ok_or_else(|| invalid_static("invalid writer chunk column boundary"))?,
            );
        }
        let raw = if columns.iter().all(|column| column.is_empty()) {
            Vec::new()
        } else {
            frame(&columns)?
        };
        let compressed = if raw.is_empty() {
            Vec::new()
        } else {
            compress(&raw)?
        };
        sections.push((id, raw.len(), compressed));
        previous.clone_from(end);
    }
    if previous != later.columns.0.iter().map(Vec::len).collect::<Vec<_>>() {
        return Err(invalid_static(
            "writer chunk columns do not cover the section",
        ));
    }
    Ok(sections)
}

pub(super) fn join(chunks: impl IntoIterator<Item = io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    let mut joined: Vec<Vec<u8>> = Vec::new();
    for raw in chunks {
        let raw = raw?;
        if raw.is_empty() {
            continue;
        }
        let columns = columns(&raw)?;
        if columns.iter().all(|column| column.is_empty()) {
            return Err(invalid_static("non-empty chunk holds no column bytes"));
        }
        if joined.is_empty() {
            joined.resize_with(columns.len(), Vec::new);
        }
        if joined.len() != columns.len() {
            return Err(invalid_static("chunk column counts disagree"));
        }
        for (joined, column) in joined.iter_mut().zip(columns) {
            engine_allocation::reserve(joined, column.len())?;
            joined.extend_from_slice(column);
        }
    }
    if joined.is_empty() {
        return Err(invalid_static("later section contains only empty chunks"));
    }
    frame(&joined)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn replace_sections(
    replay: &Replay,
    original: &BlockIndex,
    overrides: &[(usize, Vec<u8>)],
) -> io::Result<(BlockIndex, Vec<u8>)> {
    let block = replay.block_bytes(original)?;
    let directory = Directory::parse(&block, original)?;
    let mut sections = Vec::new();
    for (id, entry) in directory.sections.iter().enumerate() {
        let Some(&(offset, stored, raw)) = entry.as_ref() else {
            continue;
        };
        let mut frames = Vec::new();
        if let Some((_, replacement)) = overrides.iter().find(|(target, _)| *target == id) {
            frames.push((replacement.len(), compress(replacement)?));
            if id >= 3 && id % 2 == 1 {
                frames.resize_with(slots(original)?, || (0, Vec::new()));
            }
        } else if directory.chunks[id].is_empty() {
            frames.push((raw, block[offset..offset + stored].to_vec()));
        } else {
            for &(offset, stored, raw) in &directory.chunks[id] {
                frames.push((raw, block[offset..offset + stored].to_vec()));
            }
        }
        sections.push((id, frames));
    }
    let mut header = Vec::new();
    put_varint(&mut header, directory.frames as u128);
    put_varint(&mut header, sections.len() as u128);
    let mut raw_length = 0_u64;
    for (id, frames) in &sections {
        let raw: usize = frames.iter().map(|frame| frame.0).sum();
        let stored: usize = frames.iter().map(|frame| frame.1.len()).sum();
        put_varint(&mut header, *id as u128);
        put_varint(&mut header, raw as u128);
        put_varint(&mut header, stored as u128);
        raw_length += raw as u64;
        if *id >= 3 && *id % 2 == 1 {
            for (raw, stored) in frames {
                put_varint(&mut header, stored.len() as u128);
                put_varint(&mut header, *raw as u128);
            }
        }
    }
    let mut bytes = CHOSEN_MAGIC.to_vec();
    bytes.extend_from_slice(&crc32fast::hash(&header).to_le_bytes());
    bytes.extend_from_slice(&header);
    for (_, frames) in sections {
        for (_, stored) in frames {
            bytes.extend_from_slice(&stored);
        }
    }
    let mut index = original.clone();
    index.length = bytes.len() as u64;
    index.raw_length = raw_length;
    Ok((index, bytes))
}
