use super::{Operation, Part, encode, walk};
use serde_json::Value;
use std::{
    hash::{BuildHasher, RandomState},
    ops::Range,
};

const MAX_EDITS: usize = 32;
const WIDTH: usize = MAX_EDITS * 2 + 1;
const MAX_CELLS: usize = 256 * 1024;
const UNREACHABLE: u8 = MAX_EDITS as u8 + 1;

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

fn cell(i: usize, j: usize) -> Option<usize> {
    (i.abs_diff(j) <= MAX_EDITS).then(|| i * WIDTH + j + MAX_EDITS - i)
}

fn cost(cells: &[u8], i: usize, j: usize) -> u8 {
    cell(i, j)
        .and_then(|index| cells.get(index))
        .copied()
        .unwrap_or(UNREACHABLE)
}

fn find_hunks(
    before: &[Value],
    after: &[Value],
    mut emit: impl FnMut(Range<usize>, Range<usize>),
) -> bool {
    let (n, m) = (before.len(), after.len());
    if n.abs_diff(m) > MAX_EDITS || n.max(m) >= MAX_CELLS / WIDTH {
        return false;
    }
    let hasher = RandomState::new();
    let a: Vec<_> = before.iter().map(|value| hasher.hash_one(value)).collect();
    let b: Vec<_> = after.iter().map(|value| hasher.hash_one(value)).collect();
    let same = |i: usize, j: usize| i < n && j < m && a[i] == b[j] && before[i] == after[j];
    let mut cells = vec![UNREACHABLE; (n + 1) * WIDTH];
    for i in (0..=n).rev() {
        for j in (i.saturating_sub(MAX_EDITS)..=m.min(i + MAX_EDITS)).rev() {
            let value = if i == n && j == m {
                0
            } else if same(i, j) {
                cost(&cells, i + 1, j + 1)
            } else {
                let remove = cost(&cells, i + 1, j);
                let insert = cost(&cells, i, j + 1);
                (remove.min(insert) + 1).min(UNREACHABLE)
            };
            cells[cell(i, j).expect("search stays within its band")] = value;
        }
    }
    if cost(&cells, 0, 0) == UNREACHABLE {
        return false;
    }
    let (mut i, mut j, mut first_a, mut first_b) = (0, 0, 0, 0);
    while i < n || j < m {
        if same(i, j) {
            if i != first_a || j != first_b {
                emit(first_a..i, first_b..j);
            }
            i += 1;
            j += 1;
            (first_a, first_b) = (i, j);
        } else if i < n && (j == m || cost(&cells, i + 1, j) <= cost(&cells, i, j + 1)) {
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
