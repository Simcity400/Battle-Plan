//! An opinion owner's rows as the pairs its read proves.
use std::io;

/// Pairs by resolved target, each the last row's value, and the rows refusing the owner's proof.
pub(super) fn project<T: Ord + Copy>(
    rows: impl Iterator<Item = io::Result<(u32, i32)>>,
    resolve: impl Fn(u32) -> Option<T>,
) -> io::Result<(Vec<(T, i32)>, usize)> {
    let mut resolved = Vec::new();
    let mut unresolved = 0;
    for row in rows {
        let (target, opinion) = row?;
        match resolve(target) {
            Some(target) => resolved.push((target, resolved.len(), opinion)),
            None => unresolved += 1,
        }
    }
    resolved.sort_unstable_by_key(|&(target, at, _)| (target, at));
    let mut pairs: Vec<(T, i32)> = Vec::with_capacity(resolved.len());
    for (target, _, opinion) in resolved {
        if let Some(last) = pairs.last_mut().filter(|last| last.0 == target) {
            last.1 = opinion;
            unresolved += 1;
        } else {
            // The game lists only nonzero opinions: a zero row is not a proven pair.
            unresolved += usize::from(opinion == 0);
            pairs.push((target, opinion));
        }
    }
    Ok((pairs, unresolved))
}
