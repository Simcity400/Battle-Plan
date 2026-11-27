use super::{Operation, Part, encode, walk};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    hash::{BuildHasher, RandomState},
    iter,
    ops::Range,
};

const MAX_CELLS: usize = 256 * 1024;

pub(super) fn diff(
    before: &[Value],
    after: &[Value],
    path: &mut Vec<Part>,
    out: &mut Vec<Operation>,
) {
    let prefix = before.iter().zip(after).take_while(|(a, b)| a == b).count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let before = &before[prefix..before.len() - suffix];
    let after = &after[prefix..after.len() - suffix];
    if let Some(matches) = keyed_matches(before, after) {
        let (mut first_old, mut first_new) = (0, 0);
        for (old, new) in matches
            .into_iter()
            .rev()
            .chain(iter::once((before.len(), after.len())))
        {
            if old != first_old || new != first_new {
                out.push(Operation::Splice(
                    path.clone(),
                    prefix + first_new,
                    old - first_old,
                    after[first_new..new].to_vec(),
                ));
            }
            if old == before.len() {
                break;
            }
            path.push(Part::Index(prefix + new));
            walk(&before[old], &after[new], path, out);
            path.pop();
            (first_old, first_new) = (old + 1, new + 1);
        }
        return;
    }
    if !before.is_empty()
        && !after.is_empty()
        && find_hunks(before, after, |removed, inserted| {
            emit(
                prefix + inserted.start,
                &before[removed],
                &after[inserted],
                path,
                out,
            );
        })
    {
        return;
    }
    emit(prefix, before, after, path, out);
}

fn row_key(row: &Value) -> Option<&str> {
    row.get("key")?.as_str()
}

fn keyed_matches(before: &[Value], after: &[Value]) -> Option<Vec<(usize, usize)>> {
    row_key(before.first()?)?;
    row_key(after.first()?)?;

    let mut positions = HashMap::with_capacity(before.len());
    for (index, row) in before.iter().enumerate() {
        if positions.insert(row_key(row)?, index).is_some() {
            return None;
        }
    }
    let mut seen = HashSet::with_capacity(after.len());
    let mut pairs = Vec::new();
    for (index, row) in after.iter().enumerate() {
        let key = row_key(row)?;
        if !seen.insert(key) {
            return None;
        }
        if let Some(&old) = positions.get(key) {
            pairs.push((old, index));
        }
    }

    let mut tails: Vec<usize> = Vec::new();
    let mut predecessors = Vec::with_capacity(pairs.len());
    for (index, &(old, _)) in pairs.iter().enumerate() {
        let length = tails.partition_point(|&end| pairs[end].0 < old);
        predecessors.push(length.checked_sub(1).map(|previous| tails[previous]));
        if length == tails.len() {
            tails.push(index);
        } else {
            tails[length] = index;
        }
    }
    let mut matches = Vec::with_capacity(tails.len());
    let mut cursor = tails.last().copied();
    while let Some(index) = cursor {
        matches.push(pairs[index]);
        cursor = predecessors[index];
    }
    Some(matches)
}

fn emit(
    index: usize,
    removed: &[Value],
    inserted: &[Value],
    path: &mut Vec<Part>,
    out: &mut Vec<Operation>,
) {
    if removed.len() != inserted.len() {
        out.push(Operation::Splice(
            path.clone(),
            index,
            removed.len(),
            inserted.to_vec(),
        ));
        return;
    }
    for (offset, (before, after)) in removed.iter().zip(inserted).enumerate() {
        path.push(Part::Index(index + offset));
        let first = out.len();
        walk(before, after, path, out);
        if out.len() > first && !matches!(&out[first..], [Operation::Replace(at, _)] if at == path)
        {
            let replacement = Operation::Replace(path.clone(), after.clone());
            let nested = encode(&out[first..]).map_or(usize::MAX, |data| data.len());
            let whole =
                encode(std::slice::from_ref(&replacement)).map_or(usize::MAX, |data| data.len());
            if nested >= whole {
                out.truncate(first);
                out.push(replacement);
            }
        }
        path.pop();
    }
}

fn find_hunks(
    before: &[Value],
    after: &[Value],
    mut emit: impl FnMut(Range<usize>, Range<usize>),
) -> bool {
    let (n, m) = (before.len(), after.len());
    let width = m + 1;
    let count = (n + 1).saturating_mul(width);
    if count > MAX_CELLS {
        return false;
    }
    let hasher = RandomState::new();
    let a: Vec<_> = before.iter().map(|value| hasher.hash_one(value)).collect();
    let b: Vec<_> = after.iter().map(|value| hasher.hash_one(value)).collect();
    let same = |i: usize, j: usize| a[i] == b[j] && before[i] == after[j];
    let mut cells = vec![0_u16; count];
    for i in (0..n).rev() {
        let row = i * width;
        let next = row + width;
        for j in (0..m).rev() {
            cells[row + j] = if same(i, j) {
                cells[next + j + 1] + 1
            } else {
                cells[next + j].max(cells[row + j + 1])
            };
        }
    }
    let (mut i, mut j, mut first_a, mut first_b) = (0, 0, 0, 0);
    while i < n || j < m {
        if i < n && j < m && same(i, j) {
            if i != first_a || j != first_b {
                emit(first_a..i, first_b..j);
            }
            i += 1;
            j += 1;
            (first_a, first_b) = (i, j);
        } else if i < n && (j == m || cells[(i + 1) * width + j] >= cells[i * width + j + 1]) {
            i += 1;
        } else {
            j += 1;
        }
    }
    if i != first_a || j != first_b {
        emit(first_a..i, first_b..j);
    }
    true
}
