use super::*;

pub fn encode_changes_at<T: RowCodec>(
    changed: &[T],
    previous: &[T],
    keys: &[u128],
    rows: usize,
    output: &mut Vec<u8>,
    deleted: &mut Vec<u8>,
) -> io::Result<bool> {
    encode_at(changed, previous, keys, rows, output, deleted, None)
}

pub fn encode_changes_at_noted<T: RowCodec>(
    changed: &[T],
    previous: &[T],
    keys: &[u128],
    rows: usize,
    output: &mut Vec<u8>,
    deleted: &mut Vec<u8>,
    written: &mut Vec<(usize, usize)>,
    gone: &mut Vec<u128>,
) -> io::Result<bool> {
    written.clear();
    gone.clear();
    encode_at(
        changed,
        previous,
        keys,
        rows,
        output,
        deleted,
        Some((written, gone)),
    )
}

pub fn delta_rows(
    table: TableId,
    format: u16,
    output: &[u8],
    written: &[(usize, usize)],
) -> io::Result<Vec<DeltaRow>> {
    let fields = table.last_change().fields(table, format);
    let width = fields.div_ceil(8);
    let mut rows = allocation::vec(written.len())?;
    let mut previous_key = 0;
    for &(start, end) in written {
        let mut input = Decoder::new(&output[start..end]);
        input.format = format;
        input.recorded_fields = fields;
        let key = table.decode_key(previous_key, &mut input)?;
        let mut mask = [0; 8];
        mask[..width].copy_from_slice(input.take(width)?);
        previous_key = key;
        rows.push(DeltaRow {
            format,
            key,
            mask: u64::from_le_bytes(mask),
            recorded_fields: fields,
            values: allocation::copy_vec(input.rest())?,
        });
    }
    Ok(rows)
}

type Notes<'a> = (&'a mut Vec<(usize, usize)>, &'a mut Vec<u128>);

fn encode_at<T: RowCodec>(
    changed: &[T],
    previous: &[T],
    keys: &[u128],
    rows: usize,
    output: &mut Vec<u8>,
    deleted: &mut Vec<u8>,
    mut notes: Option<Notes<'_>>,
) -> io::Result<bool> {
    u32::try_from(rows).map_err(|_| invalid("too many rows"))?;
    u32::try_from(previous.len()).map_err(|_| invalid("too many rows"))?;
    allocation::reserve(output, 8)?;
    let count_at = output.len();
    output.extend_from_slice(&0_u32.to_le_bytes());
    let mut changed_count = 0_u32;
    let mut changed_key = 0_u128;
    deleted.clear();
    let mut deleted_count = 0_u32;
    let mut deleted_key = 0_u128;
    let mut delete = |key| -> io::Result<()> {
        allocation::reserve(deleted, (T::KEY_BITS as usize).div_ceil(7).max(1))?;
        put_varint(deleted, key - deleted_key);
        deleted_key = key;
        deleted_count += 1;
        Ok(())
    };
    let mut rows_at = changed.iter().peekable();
    let mut from = 0;
    for &key in keys {
        let row = rows_at.next_if(|row| row.key() == key);
        let baseline = match previous[from..].binary_search_by_key(&key, |row| row.key()) {
            Ok(at) => {
                from += at + 1;
                Some(&previous[from - 1])
            }
            Err(at) => {
                from += at;
                None
            }
        };
        match (row, baseline) {
            (Some(row), baseline) => {
                if baseline == Some(row) {
                    continue;
                }
                let start = output.len();
                if row.encode_change(baseline, changed_key, output)? {
                    changed_count += 1;
                    changed_key = key;
                    if let Some((written, _)) = &mut notes {
                        allocation::reserve(written, 1)?;
                        written.push((start, output.len()));
                    }
                }
            }
            (None, Some(_)) => {
                delete(key)?;
                if let Some((_, gone)) = &mut notes {
                    allocation::reserve(gone, 1)?;
                    gone.push(key);
                }
            }
            (None, None) => {}
        }
    }
    if rows_at.next().is_some() {
        return Err(invalid("changed rows outside their keys"));
    }
    output[count_at..count_at + 4].copy_from_slice(&changed_count.to_le_bytes());
    allocation::reserve(output, allocation::add(4, deleted.len())?)?;
    output.extend_from_slice(&deleted_count.to_le_bytes());
    output.extend_from_slice(deleted);
    Ok(changed_count != 0 || deleted_count != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Cases(u64);
    impl Cases {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }
    }

    fn token(key: u32, literal: u64) -> LexicalTokenRow {
        LexicalTokenRow {
            token: key,
            literal: format!("literal-{literal}").into_bytes(),
        }
    }

    fn decoded(table: TableId, body: &[u8]) -> (Vec<DeltaRow>, Vec<u128>) {
        let format = FormatVersion::CURRENT.version();
        let mut input = Decoder::new(body);
        input.format = format;
        input.recorded_fields = table.last_change().fields(table, format);
        let mut previous = 0;
        let rows = (0..input.u32().unwrap())
            .map(|_| {
                let row = decode_delta(table, previous, &mut input).unwrap();
                previous = row.key;
                row
            })
            .collect();
        let mut previous = 0;
        let gone = (0..input.u32().unwrap())
            .map(|_| {
                previous = table.decode_key(previous, &mut input).unwrap();
                previous
            })
            .collect();
        input.finish().unwrap();
        (rows, gone)
    }

    fn agree<T: RowCodec>(table: TableId, rows: &[T], previous: &[T], keys: &[u128]) {
        let mut whole = vec![7];
        let mut whole_deleted = vec![9];
        let whole_result = encode_changes_typed(rows, previous, &mut whole, &mut whole_deleted, None)
            .map_err(|e| e.to_string());
        let changed: Vec<T> = rows
            .iter()
            .filter(|row| keys.binary_search(&row.key()).is_ok())
            .cloned()
            .collect();
        let mut at = vec![7];
        let mut at_deleted = vec![9];
        let at_result = encode_changes_at(
            &changed,
            previous,
            keys,
            rows.len(),
            &mut at,
            &mut at_deleted,
        )
        .map_err(|e| e.to_string());
        assert_eq!(whole_result, at_result, "keys {keys:?}");
        assert_eq!(whole, at, "keys {keys:?}");
        assert_eq!(whole_deleted, at_deleted, "keys {keys:?}");
        let mut noted = vec![7];
        let (mut written, mut gone) = (vec![(1, 1)], vec![1]);
        let noted_result = encode_changes_at_noted(
            &changed,
            previous,
            keys,
            rows.len(),
            &mut noted,
            &mut Vec::new(),
            &mut written,
            &mut gone,
        )
        .map_err(|e| e.to_string());
        assert_eq!(noted_result, at_result, "keys {keys:?}");
        if at_result.is_ok() {
            assert_eq!(noted, at, "keys {keys:?}");
            let format = FormatVersion::CURRENT.version();
            let direct = delta_rows(table, format, &noted, &written).unwrap();
            assert_eq!((direct, gone), decoded(table, &noted[1..]), "keys {keys:?}");
        }
    }

    #[test]
    fn changes_at_candidate_keys_equal_the_whole_table_merge() {
        let mut cases = Cases(0x5eed);
        for _ in 0..2_000 {
            let previous: Vec<_> = (1..48)
                .filter(|_| cases.next(3) != 0)
                .map(|key| token(key, u64::from(key)))
                .collect();
            let mut keys = Vec::new();
            let mut rows = Vec::new();
            let mut held = previous.iter().peekable();
            for key in 0..52_u32 {
                let before = held.next_if(|row| row.token == key);
                let candidate = cases.next(4) == 0;
                let row = match (before, candidate) {
                    (Some(before), false) => Some(before.clone()),
                    (before, true) => match cases.next(4) {
                        0 => None,
                        1 => before.cloned(),
                        _ => Some(token(key, 1_000 + cases.next(5))),
                    },
                    (None, false) => None,
                };
                if candidate {
                    keys.push(u128::from(key));
                }
                rows.extend(row);
            }
            agree(TableId::LexicalTokens, &rows, &previous, &keys);
        }
        let previous: Vec<_> = (1..6).map(|key| token(key, 0)).collect();
        agree(TableId::LexicalTokens, &previous, &previous, &[]);
        let mut rows = previous.clone();
        rows[0] = token(1, 5);
        rows[4] = token(5, 5);
        agree(TableId::LexicalTokens, &rows, &previous, &[1, 5]);
        agree(TableId::LexicalTokens, &rows[1..], &previous, &[1, 5]);
        let mut output = Vec::new();
        assert!(
            encode_changes_at(
                &[token(3, 9)],
                &previous,
                &[2],
                5,
                &mut output,
                &mut Vec::new()
            )
            .is_err()
        );
    }
}
