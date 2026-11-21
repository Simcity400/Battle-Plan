use super::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

pub(super) type Snapshot = Option<Arc<rows::TableRows>>;
#[derive(Default)]
struct Cache {
    entries: Vec<rows::VariantBonusSnapshotRow>,
    indices: HashMap<u64, usize>,
    published: Snapshot,
    dirty: bool,
    payload_bytes: usize,
    published_bytes: usize,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE
        .get_or_init(|| Mutex::new(Cache::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Cache {
    fn intern(&mut self, source: &rows::VariantBonusSource) -> Result<u64, &'static str> {
        if source
            .stats
            .iter()
            .any(|entry| entry.index >= cequipment_variant::STAT_COUNT as u32)
        {
            return Err("applied design bonus stat index outside native metadata");
        }
        let id = rows::variant_bonus_snapshot_id(source);
        if let Some(index) = self.indices.get(&id) {
            return if self.entries[*index].source == *source {
                Ok(id)
            } else {
                Err("applied design bonus snapshot identity collision")
            };
        }
        engine_allocation::reserve_map(&mut self.indices, 1)
            .map_err(|_| "design bonus dictionary allocation failed")?;
        engine_allocation::reserve(&mut self.entries, 1)
            .map_err(|_| "design bonus dictionary allocation failed")?;
        let source = rows::NativeValue::try_clone(source)
            .map_err(|_| "design bonus dictionary allocation failed")?;
        self.payload_bytes = self
            .payload_bytes
            .saturating_add(rows::NativeValue::heap_bytes(&source));
        self.indices.insert(id, self.entries.len());
        self.entries
            .push(rows::VariantBonusSnapshotRow { id, source });
        self.dirty = true;
        Ok(id)
    }

    fn snapshot(&mut self) -> Result<Snapshot, &'static str> {
        if self.dirty || self.published.is_none() {
            let mut rows = engine_allocation::vec(self.entries.len())
                .map_err(|_| "design bonus dictionary allocation failed")?;
            let mut bytes = rows.capacity() * std::mem::size_of::<rows::VariantBonusSnapshotRow>();
            for row in &self.entries {
                let row = rows::RowCodec::try_clone(row)
                    .map_err(|_| "design bonus dictionary allocation failed")?;
                bytes = bytes.saturating_add(rows::RowCodec::heap_bytes(&row));
                rows.push(row);
            }
            rows.sort_unstable_by_key(|row| row.id);
            self.published = Some(Arc::new(rows::TableRows::VariantBonusSnapshots(rows)));
            self.dirty = false;
            self.published_bytes = bytes;
        }
        Ok(self.published.clone())
    }
}

pub(super) fn intern_source(source: &rows::VariantBonusSource) -> Result<u64, &'static str> {
    cache().intern(source)
}

pub(super) fn intern(
    state: cequipment_variant::VariantDesignState,
) -> Result<rows::VariantDesignState, &'static str> {
    if state.bonus_sources.is_empty() && state.named_sources.is_empty() {
        return Ok(state.with_sources(Vec::new(), Vec::new()));
    }
    let mut ids = engine_allocation::vec(state.bonus_sources.len())
        .map_err(|_| "design bonus reference allocation failed")?;
    let mut cache = cache();
    for source in &state.bonus_sources {
        ids.push(cache.intern(source)?);
    }
    let mut named = engine_allocation::vec(state.named_sources.len())
        .map_err(|_| "design bonus reference allocation failed")?;
    for source in &state.named_sources {
        named.push(cache.intern(source)?);
    }
    Ok(state.with_sources(ids, named))
}

pub(super) fn snapshot() -> Result<Snapshot, &'static str> {
    cache().snapshot()
}
pub(super) fn input(snapshot: &Snapshot) -> TableInput {
    match snapshot {
        Some(rows) => TableInput::SharedComplete(Arc::clone(rows)),
        None => unavailable("design bonus snapshots not acquired").into(),
    }
}

pub(super) fn latest(a: &Snapshot, b: &Snapshot) -> Snapshot {
    match (a, b) {
        (Some(a), Some(b)) => Some(Arc::clone(if a.len() >= b.len() { a } else { b })),
        (a, b) => a.clone().or_else(|| b.clone()),
    }
}

pub(super) fn release() {
    if let Some(cache) = CACHE.get() {
        *cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Cache::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_bonus_snapshots_share_exact_values_and_keep_older_applied_versions() {
        let mut cache = Cache::default();
        let mut source = rows::VariantBonusSource {
            idea: Some(b"synthetic_company".to_vec()),
            idea_token: Some(9),
            stats: vec![rows::VariantStatEntry {
                index: 0,
                value: 20_000,
            }],
            ..Default::default()
        };
        let id = cache.intern(&source).unwrap();
        assert_eq!(cache.intern(&source).unwrap(), id);
        let first = cache.snapshot().unwrap().unwrap();
        assert_eq!(first.len(), 1);
        let quiet = collector_test_support::with_allocations_denied(|| cache.snapshot())
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&first, &quiet));
        source.stats[0].value = 30_000;
        assert_ne!(cache.intern(&source).unwrap(), id);
        let second = cache.snapshot().unwrap().unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 2);
        assert_eq!(
            first.rows::<tables::VariantBonusSnapshots>().unwrap()[0]
                .source
                .stats[0]
                .value,
            20_000
        );
    }
}
