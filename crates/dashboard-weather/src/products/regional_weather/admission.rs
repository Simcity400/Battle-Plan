use dashboard_base::prepared::facts::{Credit, MemoryBudget};
use std::io;

pub(super) fn pressure(owner: &'static str, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("regional-weather {owner}: {error}"))
}

pub(super) fn add(left: usize, right: usize) -> io::Result<usize> {
    left.checked_add(right)
        .ok_or_else(|| pressure("allocation size", io::ErrorKind::OutOfMemory.into()))
}

pub(super) fn bytes<T>(count: usize) -> io::Result<usize> {
    count
        .checked_mul(size_of::<T>())
        .ok_or_else(|| pressure("allocation size", io::ErrorKind::OutOfMemory.into()))
}

pub(super) fn admit(bytes: usize, owner: &'static str) -> io::Result<Credit> {
    admit_in(&MemoryBudget::preparation(), bytes, owner)
}

pub(super) fn admit_in(
    budget: &MemoryBudget,
    bytes: usize,
    owner: &'static str,
) -> io::Result<Credit> {
    budget.admit(bytes).map_err(|error| pressure(owner, error))
}

pub(super) fn vector<T>(count: usize, owner: &'static str) -> io::Result<(Vec<T>, Credit)> {
    let credit = admit(bytes::<T>(count)?, owner)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|error| pressure(owner, io::Error::new(io::ErrorKind::OutOfMemory, error)))?;
    Ok((values, credit))
}

// One maximum 11-slot B-tree node per entry covers even singleton/split nodes.
pub(super) fn tree_entry<K, V>() -> io::Result<usize> {
    add(160, bytes::<(K, V)>(11)?)
}

pub(super) fn name(id: u32, measure: &str) -> io::Result<String> {
    use std::fmt::Write;
    let mut name = String::new();
    name.try_reserve_exact(11 + measure.len())
        .map_err(|error| {
            pressure(
                "region name",
                io::Error::new(io::ErrorKind::OutOfMemory, error),
            )
        })?;
    write!(&mut name, "{id}|{measure}").unwrap();
    Ok(name)
}
