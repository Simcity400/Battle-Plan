//! Consumer fixture: valid section framing with invalid unused column grammar.
use super::*;

pub(crate) fn trailing_control(
    replay: &Replay,
    table: TableId,
) -> io::Result<(BlockIndex, Vec<u8>)> {
    let original = replay
        .index
        .first()
        .ok_or_else(|| invalid("fixture has no block"))?;
    let bytes = replay.block_bytes(original)?;
    let directory = Directory::parse(&bytes, original)?;
    let target = later_section(table);
    if directory.sections[target].is_none() {
        return Err(invalid("fixture needs a later table section"));
    }
    let mut sections = Vec::new();
    for (id, section) in directory.sections.iter().enumerate() {
        let Some((offset, length, raw_length)) = section else {
            continue;
        };
        if id == target {
            let raw = directory.raw(&bytes, id)?.unwrap();
            let reader =
                ColumnReader::stored(&raw, table.field_count(), table, replay.format.version())?;
            let mut columns = Columns::new(table.field_count());
            for (out, mut input) in columns.0.iter_mut().zip(reader.0) {
                out.extend_from_slice(input.take(input.remaining())?);
            }
            if let Some(marks) = reader.1 {
                columns.marks(table, marks)?;
            }
            columns.0[CONTROL].push(255);
            let raw = columns.raw();
            sections.push((id, raw.len(), compress(&raw)?));
        } else {
            sections.push((id, *raw_length, bytes[*offset..*offset + *length].to_vec()));
        }
    }
    let mut header = Vec::new();
    put_varint(&mut header, directory.frames as u128);
    put_varint(&mut header, sections.len() as u128);
    for (id, raw, stored) in &sections {
        put_varint(&mut header, *id as u128);
        put_varint(&mut header, *raw as u128);
        put_varint(&mut header, stored.len() as u128);
    }
    let mut bytes = directory.layout.magic().to_vec();
    bytes.extend_from_slice(&crc32fast::hash(&header).to_le_bytes());
    bytes.extend_from_slice(&header);
    let raw_length = sections.iter().map(|(_, raw, _)| *raw as u64).sum();
    for (_, _, stored) in sections {
        bytes.extend_from_slice(&stored);
    }
    let mut index = original.clone();
    index.length = bytes.len() as u64;
    index.raw_length = raw_length;
    Ok((index, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_fixture_keeps_prefix_values_and_exact_full_read_refusal_with_any_cache() {
        let good = test_recording::build(b"synthetic-prefix", &[10, 12, 13], |hour, _| {
            let mut table_ns = [0; TABLE_COUNT];
            table_ns[TableId::Clock.index()] = 1;
            Metrics {
                total_ns: hour as u64,
                table_ns,
                ..Default::default()
            }
        });
        let selected = &[TableId::Campaign, TableId::Clock];
        let oracle = Replay::open(good.bytes().to_vec()).unwrap();
        oracle.cache_decoded_sections(false);
        let expected = oracle
            .read_block_selected(&oracle.index[0], selected)
            .unwrap();
        for cache in [false, true] {
            for warm in [false, true] {
                let bad = super::super::super::test_work::with_trailing_table_control(
                    &good,
                    TableId::Clock,
                )
                .unwrap();
                bad.cache_decoded_sections(cache);
                let index = &bad.index[0];
                if warm {
                    assert_eq!(
                        bad.read_block_selected_through(index, Some(selected), 12, None)
                            .unwrap(),
                        expected[..2]
                    );
                }
                for workers in [1, 2] {
                    let parallel = Parallel {
                        workers,
                        on_worker: &|| {},
                    };
                    let error = bad
                        .read_block_selected_through(index, Some(selected), 13, Some(&parallel))
                        .unwrap_err();
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                    assert_eq!(error.to_string(), "grouped section has unread column bytes");
                    assert!(!bad.is_verified(index));
                    assert_eq!(
                        bad.read_block_selected_through(index, Some(selected), 12, Some(&parallel))
                            .unwrap(),
                        expected[..2]
                    );
                }
            }
        }
    }
}
