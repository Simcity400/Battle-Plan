use super::*;
use crate::prepared::facts::{
    Limits as FactLimits, MapCheckpoint, Namespace, PagedMap, ReadPages, ReadPhase,
};
use map_display_data::credit::{MemoryBudget, display_products};

type Index = PagedMap<Hour, u64>;
fn namespace(key: &Key, branch: BranchId) -> io::Result<Namespace> {
    let family = key
        .digest()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Namespace::new(&family, branch, "map-export-endpoints")
}
fn limits(budget: &MemoryBudget) -> FactLimits {
    FactLimits {
        write_bytes: budget.limit(),
        cache_bytes: 0,
        page_bytes: budget.limit(),
    }
}
fn section(key: &Key, branch: BranchId) -> String {
    key.section(&format!("endpoint-index/{branch}"))
}
pub fn put(
    writer: &mut Writer,
    key: &Key,
    branch: BranchId,
    hour: Hour,
    ordinal: u64,
) -> io::Result<()> {
    let owner = display_products();
    let root = section(key, branch);
    let checkpoint = if writer.read_at(&root, 0)?.is_some() {
        let phase = ReadPhase::new(writer, owner.clone(), 0)?;
        Some(MapCheckpoint::read(&phase, &root)?)
    } else {
        None
    };
    let mut index = if let Some(checkpoint) = checkpoint {
        if *checkpoint.namespace() != namespace(key, branch)? || checkpoint.cursor != ordinal {
            return Err(invalid(
                "map export endpoint checkpoint differs from its publication",
            ));
        }
        Index::restore(&checkpoint, owner.clone(), limits(owner))?
    } else {
        if ordinal != 0 {
            return Err(invalid("map export endpoint keyed index is missing"));
        }
        Index::new(namespace(key, branch)?, owner.clone(), limits(owner))?
    };
    index.put(writer, hour, ordinal)?;
    index
        .checkpoint(
            ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("map export endpoint count overflow"))?,
        )?
        .write(writer, &root, owner)
}
pub fn get(
    snapshot: &mut store::Snapshot,
    key: &Key,
    branch: BranchId,
    hour: Hour,
) -> io::Result<Option<u64>> {
    let count = snapshot.range_count(&key.records(branch)).unwrap_or(0);
    get_in(snapshot, key, branch, hour, count)
}
pub(super) fn get_in<R: ReadPages>(
    source: &mut R,
    key: &Key,
    branch: BranchId,
    hour: Hour,
    count: u64,
) -> io::Result<Option<u64>> {
    let owner = display_products();
    if count == 0 {
        return Ok(None);
    }
    let phase = ReadPhase::new(source, owner.clone(), 0)?;
    let checkpoint = MapCheckpoint::read(&phase, &section(key, branch))?;
    if *checkpoint.namespace() != namespace(key, branch)? || checkpoint.cursor != count {
        return Err(invalid(
            "map export endpoint checkpoint differs from its publication",
        ));
    }
    let index = Index::restore(&checkpoint, owner.clone(), limits(owner))?;
    if index.len() != count {
        return Err(invalid("map export endpoint keyed count differs"));
    }
    Ok(index.get(&phase, &hour)?.map(|ordinal| *ordinal))
}
