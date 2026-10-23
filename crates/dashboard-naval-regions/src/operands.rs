//! Lossless, sparse prepared diplomacy operands. Each row uses its native
//! codec's changed fields from an empty baseline, packed into numeric words.
//! No frame, row strings or historical packets grow the observer catalogue.
use dashboard_keyed::{Delta, Values};
use replay_data::rows::{DeltaRow, RowCodec, TableId, TableSchema};
use replay_frame::frame::{DecodedFrame, SharedView, TableView};
use std::{collections::BTreeMap, io};

pub fn save<T: TableSchema>(
    frame: &DecodedFrame,
    values: &mut Values,
    reduce: impl Fn(&T::Row) -> T::Row,
) -> io::Result<()> {
    let Ok(rows) = frame.complete::<T>() else {
        return Ok(());
    };
    values.push(known::<T>(), 1);
    for row in rows {
        save_row::<T>(&reduce(row), &mut |name, value| values.push(name, value))?;
    }
    Ok(())
}

/// The name marking `T`'s rows known.
pub fn known<T: TableSchema>() -> String {
    format!("input|{}|known", T::ID.index())
}

/// The values one reduced row of `T` saves, in the order [`save`] pushes them.
pub fn save_row<T: TableSchema>(
    row: &T::Row,
    push: &mut dyn FnMut(String, i128),
) -> io::Result<()> {
    let delta = row
        .delta(None)?
        .ok_or_else(|| io::Error::other("Prepared naval row has no insertion"))?;
    let name = format!("input|{}|{}", T::ID.index(), delta.key);
    push(format!("{name}|length"), delta.values.len() as i128);
    push(format!("{name}|fields"), delta.recorded_fields as i128);
    push(format!("{name}|mask"), i128::from(delta.mask));
    for (at, chunk) in delta.values.chunks(8).enumerate() {
        let mut word = [0; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        push(format!("{name}|{at}"), i128::from(u64::from_le_bytes(word)));
    }
    Ok(())
}

/// `T`'s saved values' changes from `before`'s rows to `frame`'s; None when keys are out of order.
pub fn save_changes<T: TableSchema>(
    before: Option<&SharedView>,
    frame: &DecodedFrame,
    reduce: impl Fn(&T::Row) -> T::Row,
    same: impl Fn(&T::Row, &T::Row) -> bool,
    delta: &mut Delta,
) -> io::Result<Option<()>> {
    let view = &frame.tables[T::ID.index()];
    if before.is_some_and(|before| SharedView::ptr_eq(before, view)) {
        return Ok(Some(()));
    }
    let old = before.and_then(dashboard_keyed::delta::rows::<T>);
    let new = frame.complete::<T>().ok();
    let mut removed = Vec::new();
    let mut set = Vec::new();
    let mut failed = None;
    let known_now = match (old, new) {
        (Some(_), None) => Some(false),
        (None, Some(_)) => Some(true),
        _ => None,
    };
    let mut visit = |old: Option<&T::Row>, new: Option<&T::Row>| {
        if failed.is_some() {
            return;
        }
        let result = old
            .map_or(Ok(()), |row| {
                save_row::<T>(&reduce(row), &mut |name, _| removed.push(name))
            })
            .and_then(|()| {
                new.map_or(Ok(()), |row| {
                    save_row::<T>(&reduce(row), &mut |name, value| set.push((name, value)))
                })
            });
        if let Err(why) = result {
            failed = Some(why);
        }
    };
    match (old, new) {
        (None, None) => {}
        (Some(old), None) => {
            for row in old {
                visit(Some(row), None);
            }
        }
        (None, Some(new)) => {
            for row in new {
                visit(None, Some(row));
            }
        }
        (Some(old), Some(new)) => {
            if dashboard_keyed::delta::changed_rows(old, new, &same, &mut visit).is_none() {
                return Ok(None);
            }
        }
    }
    if let Some(why) = failed {
        return Err(why);
    }
    match known_now {
        Some(false) => delta.remove(known::<T>()),
        Some(true) => delta.set(known::<T>(), 1),
        None => {}
    }
    for name in removed {
        delta.remove(name);
    }
    for (name, value) in set {
        delta.set(name, value);
    }
    Ok(Some(()))
}

/// The names of `table`'s operands all begin with this.
pub fn prefix(table: TableId) -> String {
    format!("input|{}|", table.index())
}

/// The rows of `T` that `values` holds, None where the table is not known. `bound`
/// limits each row's words: the number of values the hour holds of every table.
pub fn rows<T: TableSchema>(
    values: &BTreeMap<&str, i128>,
    bound: usize,
) -> io::Result<Option<Vec<T::Row>>> {
    let prefix = prefix(T::ID);
    if values.get(format!("{prefix}known").as_str()) != Some(&1) {
        return Ok(None);
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
        if words(length) > bound {
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
    Ok(Some(rows))
}

/// The words a row of `length` bytes is saved in.
fn words(length: usize) -> usize {
    length.div_ceil(8)
}

/// The most words any row of `T` in `values` declares; zero where the table is not
/// known, as [`rows`] then reads none.
pub fn most_words<T: TableSchema>(values: &BTreeMap<&str, i128>) -> usize {
    let prefix = prefix(T::ID);
    if values.get(format!("{prefix}known").as_str()) != Some(&1) {
        return 0;
    }
    values
        .iter()
        .filter(|(name, _)| {
            name.strip_prefix(&prefix)
                .is_some_and(|rest| rest.ends_with("|length"))
        })
        .filter_map(|(_, &length)| usize::try_from(length).ok())
        .map(words)
        .max()
        .unwrap_or(0)
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
