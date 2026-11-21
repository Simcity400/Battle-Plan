
use serde::{Deserialize, Serialize};
pub fn serialize<'a, S, M, K: Serialize + 'a, V: Serialize + 'a>(
    map: &'a M,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    &'a M: IntoIterator<Item = (&'a K, &'a V)>,
{
    let mut entries = map
        .into_iter()
        .map(|(key, value)| {
            serde_json::to_vec(key)
                .map(|encoded| (encoded, key, value))
                .map_err(serde::ser::Error::custom)
        })
        .collect::<Result<Vec<_>, S::Error>>()?;
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    serializer.collect_seq(entries.into_iter().map(|(_, key, value)| (key, value)))
}
pub use serialize as ordered_pairs;
pub fn ordered_entries<K: Ord, V, H>(map: &std::collections::HashMap<K, V, H>) -> Vec<(&K, &V)> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_unstable_by_key(|(key, _)| *key);
    entries
}
pub fn ordered_map<S: serde::Serializer, K: Serialize + Ord, V: Serialize, H>(
    map: &std::collections::HashMap<K, V, H>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_map(ordered_entries(map))
}
struct OrderedMap<'a, K, V>(&'a std::collections::HashMap<K, V>);
impl<K: Serialize + Ord, V: Serialize> Serialize for OrderedMap<'_, K, V> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ordered_map(self.0, serializer)
    }
}
pub fn ordered_optional_map<S: serde::Serializer, K: Serialize + Ord, V: Serialize>(
    map: &Option<std::collections::HashMap<K, V>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match map {
        Some(map) => serializer.serialize_some(&OrderedMap(map)),
        None => serializer.serialize_none(),
    }
}
pub fn ordered_shared_optional_map<S: serde::Serializer, K: Serialize + Ord, V: Serialize>(
    map: &Option<std::sync::Arc<std::collections::HashMap<K, V>>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match map {
        Some(map) => serializer.serialize_some(&OrderedMap(map)),
        None => serializer.serialize_none(),
    }
}
pub fn ordered_set<S: serde::Serializer, K: Serialize + Ord>(
    set: &std::collections::HashSet<K>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut entries: Vec<_> = set.iter().collect();
    entries.sort_unstable();
    serializer.collect_seq(entries)
}
struct OrderedSet<'a, K>(&'a std::collections::HashSet<K>);
impl<K: Serialize + Ord> Serialize for OrderedSet<'_, K> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ordered_set(self.0, serializer)
    }
}
pub fn ordered_sets<S: serde::Serializer, K: Serialize + Ord, V: Serialize + Ord>(
    map: &std::collections::BTreeMap<K, std::collections::HashSet<V>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_map(map.iter().map(|(key, set)| (key, OrderedSet(set))))
}
pub fn deserialize<'de, D, M, K, V>(deserializer: D) -> Result<M, D::Error>
where
    D: serde::Deserializer<'de>,
    K: Deserialize<'de>,
    V: Deserialize<'de>,
    M: FromIterator<(K, V)>,
{
    Ok(Vec::<(K, V)>::deserialize(deserializer)?
        .into_iter()
        .collect())
}
pub mod shared {
    use std::{collections::HashMap, hash::Hash, sync::Arc};
    pub fn serialize<S, K, V>(map: &Arc<HashMap<K, V>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
        K: serde::Serialize,
        V: serde::Serialize,
    {
        super::serialize(map.as_ref(), serializer)
    }
    pub fn deserialize<'de, D, K, V>(deserializer: D) -> Result<Arc<HashMap<K, V>>, D::Error>
    where
        D: serde::Deserializer<'de>,
        K: serde::Deserialize<'de> + Eq + Hash,
        V: serde::Deserialize<'de>,
    {
        super::deserialize(deserializer).map(Arc::new)
    }
}
