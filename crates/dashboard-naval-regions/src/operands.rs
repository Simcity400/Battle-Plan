//! Lossless, sparse prepared diplomacy operands. Each row uses its native
//! codec's changed fields from an empty baseline, packed into numeric words.
//! No frame, row strings or historical packets grow the observer catalogue.
use dashboard_keyed::Values;
use replay_data::rows::{DeltaRow, RowCodec, TableSchema};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{collections::BTreeMap, io};

pub fn save<T: TableSchema>(
    frame: &DecodedFrame,
    values: &mut Values,
    reduce: impl Fn(&T::Row) -> T::Row,
) -> io::Result<()> {
    let Ok(rows) = frame.complete::<T>() else {
        return Ok(());
    };
    let prefix = format!("input|{}", T::ID.index());
    values.push(format!("{prefix}|known"), 1);
    for row in rows {
        let row = reduce(row);
        let delta = row
            .delta(None)?
            .ok_or_else(|| io::Error::other("Prepared naval row has no insertion"))?;
        let name = format!("{prefix}|{}", delta.key);
        values.push(format!("{name}|length"), delta.values.len() as i128);
        values.push(format!("{name}|fields"), delta.recorded_fields as i128);
        values.push(format!("{name}|mask"), i128::from(delta.mask));
        for (at, chunk) in delta.values.chunks(8).enumerate() {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            values.push(format!("{name}|{at}"), i128::from(u64::from_le_bytes(word)));
        }
    }
    Ok(())
}

pub fn restore<T: TableSchema>(
    frame: &mut DecodedFrame,
    values: &BTreeMap<&str, i128>,
) -> io::Result<()> {
    let prefix = format!("input|{}|", T::ID.index());
    if values.get(format!("{prefix}known").as_str()) != Some(&1) {
        return Ok(());
    }
    let mut rows = Vec::new();
    for (&name, &length) in values {
        let Some(key) = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix("|length"))
        else {
            continue;
        };
        let key = key.parse::<u128>().map_err(io::Error::other)?;
        let name = format!("{prefix}{key}");
        let length = usize::try_from(length).map_err(io::Error::other)?;
        let get = |field: &str| {
            values
                .get(format!("{name}|{field}").as_str())
                .copied()
                .ok_or_else(|| io::Error::other("Incomplete prepared naval operand"))
        };
        // The series catalogue, not an untrusted declared length, bounds allocation.
        if length.div_ceil(8) > values.len() {
            return Err(io::Error::other("Invalid prepared naval operand length"));
        }
        let mut bytes = Vec::new();
        bytes.try_reserve(length).map_err(io::Error::other)?;
        for at in 0..length.div_ceil(8) {
            let word = u64::try_from(get(&at.to_string())?).map_err(io::Error::other)?;
            let take = (length - bytes.len()).min(8);
            bytes.extend_from_slice(&word.to_le_bytes()[..take]);
        }
        let delta = DeltaRow {
            format: replay_data::FORMAT_VERSION,
            key,
            mask: u64::try_from(get("mask")?).map_err(io::Error::other)?,
            values: bytes,
            recorded_fields: usize::try_from(get("fields")?).map_err(io::Error::other)?,
        };
        rows.push(T::Row::apply_delta(&delta, None)?);
    }
    rows.sort_by_key(RowCodec::key);
    frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows)).into();
    Ok(())
}

pub fn blank(branch: u64, hour: i32) -> DecodedFrame {
    DecodedFrame {
        branch,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "Naval operand is not recorded".into(),
            }
            .into()
        }),
    }
}
pub fn input_name(name: &str) -> bool {
    name.starts_with("input|")
}
