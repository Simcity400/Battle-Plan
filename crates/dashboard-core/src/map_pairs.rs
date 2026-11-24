use serde::Serialize;

pub fn ordered_map<S: serde::Serializer, K: Serialize + Ord, V: Serialize, H>(
    map: &std::collections::HashMap<K, V, H>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_unstable_by_key(|(key, _)| *key);
    serializer.collect_map(entries)
}
