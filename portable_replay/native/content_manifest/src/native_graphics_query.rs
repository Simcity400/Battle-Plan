//! Recording-authenticated input to the native `interface/.gfx` query.
//! This sequence is query order, not threaded parser/registration order.

use crate::{
    ContentLayerKind, LayerStorageKind, RecordedContentLayer, SourceGameIdentity, UsedContentKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Write;

pub const NATIVE_GRAPHICS_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const QUERY_LAW: &str = "hoi4.a729.native-interface-gfx-query.v1";
pub const MAX_QUERY_ENTRIES: usize = crate::content_namespace::MAX_NAMESPACE_REFERENCES;
pub const MAX_QUERY_BYTES: u64 = crate::content_namespace::MAX_NAMESPACE_ROSTER_BYTES;
pub const MAX_QUERY_PAYLOAD_BYTES: u64 = crate::content_namespace::MAX_NAMESPACE_PAYLOAD_BYTES;
pub const MAX_QUERY_DEPTH: usize = 128;
pub const MAX_QUERY_PATH_BYTES: usize = 4096;

/// Portable refusals deliberately contain no machine-local error text or paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeGraphicsQueryUnavailable {
    NotObserved,
    UnsupportedBuild,
    UnsupportedHost,
    BoundsExceeded,
    InputUnavailable,
    UnprovenEntryKind,
    UnprovenReplacement,
    UnprovenPathSpelling,
    ChangedDuringObservation,
}
impl std::fmt::Display for NativeGraphicsQueryUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native_graphics_query_unavailable: {self:?}")
    }
}
impl std::error::Error for NativeGraphicsQueryUnavailable {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGraphicsQuerySpec {
    law: String,
    executable_sha256: String,
    directory: String,
    suffix: String,
    #[serde(deserialize_with = "required_contains")]
    contains: Option<String>,
    depth: i32,
}
fn required_contains<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::deserialize(d)
}
impl NativeGraphicsQuerySpec {
    fn current() -> Self {
        Self {
            law: QUERY_LAW.into(),
            executable_sha256: NATIVE_GRAPHICS_EXECUTABLE_SHA256.into(),
            directory: "interface".into(),
            suffix: ".gfx".into(),
            contains: None,
            depth: -1,
        }
    }
    pub fn law(&self) -> &str {
        &self.law
    }
    pub fn executable_sha256(&self) -> &str {
        &self.executable_sha256
    }
    pub fn directory(&self) -> &str {
        &self.directory
    }
    pub fn suffix(&self) -> &str {
        &self.suffix
    }
    pub fn contains(&self) -> Option<&str> {
        self.contains.as_deref()
    }
    pub fn depth(&self) -> i32 {
        self.depth
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeGraphicsQueryLayer {
    pub ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    pub storage_kind: LayerStorageKind,
    pub source_container_sha256: Option<String>,
    #[serde(deserialize_with = "bounded_sequence")]
    pub replace_paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGraphicsQueryEntry {
    pub(crate) original_query_spelling: String,
    pub(crate) content: UsedContentKey,
}
impl NativeGraphicsQueryEntry {
    pub fn original_query_spelling(&self) -> &str {
        &self.original_query_spelling
    }
    pub fn content(&self) -> &UsedContentKey {
        &self.content
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeGraphicsQueryComplete {
    specification: NativeGraphicsQuerySpec,
    authority_manifest_sha256: String,
    #[serde(deserialize_with = "bounded_sequence")]
    layers: Vec<NativeGraphicsQueryLayer>,
    /// Commitment to every per-layer directory callback, including names that
    /// do not emit a payload. Re-observation must agree before sealing.
    scope_sha256: String,
    #[serde(deserialize_with = "bounded_sequence")]
    entries: Vec<NativeGraphicsQueryEntry>,
    sequence_sha256: String,
}
impl NativeGraphicsQueryComplete {
    pub fn specification(&self) -> &NativeGraphicsQuerySpec {
        &self.specification
    }
    pub fn entries(&self) -> &[NativeGraphicsQueryEntry] {
        &self.entries
    }
    pub fn scope_sha256(&self) -> &str {
        &self.scope_sha256
    }
    pub fn sequence_sha256(&self) -> &str {
        &self.sequence_sha256
    }
    fn digest(&self) -> Result<String, String> {
        bounded_digest(&(
            QUERY_LAW,
            &self.specification,
            &self.authority_manifest_sha256,
            &self.layers,
            &self.scope_sha256,
            &self.entries,
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum QueryState {
    Unavailable {
        cause: NativeGraphicsQueryUnavailable,
    },
    Complete {
        query: NativeGraphicsQueryComplete,
    },
}

/// Wire data, not an admission capability. Complete construction belongs to the
/// bounded VFS observer; deserialization must pass manifest/playback validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NativeGraphicsQueryWitness(QueryState);
impl Default for NativeGraphicsQueryWitness {
    fn default() -> Self {
        Self::unavailable(NativeGraphicsQueryUnavailable::NotObserved)
    }
}
impl NativeGraphicsQueryWitness {
    pub fn unavailable(cause: NativeGraphicsQueryUnavailable) -> Self {
        Self(QueryState::Unavailable { cause })
    }
    pub fn unavailable_cause(&self) -> Option<&NativeGraphicsQueryUnavailable> {
        match &self.0 {
            QueryState::Unavailable { cause } => Some(cause),
            _ => None,
        }
    }
    pub fn complete(&self) -> Option<&NativeGraphicsQueryComplete> {
        match &self.0 {
            QueryState::Complete { query } => Some(query),
            _ => None,
        }
    }
    pub fn required_content(&self) -> Vec<UsedContentKey> {
        self.complete()
            .into_iter()
            .flat_map(|q| q.entries.iter().map(|e| e.content.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    /// Even a layer emitting no payload influences negative query completeness.
    pub fn required_layer_ordinals(&self) -> impl Iterator<Item = u32> + '_ {
        self.complete()
            .into_iter()
            .flat_map(|q| q.layers.iter().map(|l| l.ordinal))
    }
    pub(crate) fn from_observation(
        authority_manifest_sha256: String,
        layers: Vec<NativeGraphicsQueryLayer>,
        scope_sha256: String,
        entries: Vec<NativeGraphicsQueryEntry>,
    ) -> Result<Self, String> {
        let mut query = NativeGraphicsQueryComplete {
            specification: NativeGraphicsQuerySpec::current(),
            authority_manifest_sha256,
            layers,
            scope_sha256,
            entries,
            sequence_sha256: String::new(),
        };
        query.sequence_sha256 = query.digest()?;
        Ok(Self(QueryState::Complete { query }))
    }
    pub(crate) fn validate(
        &self,
        source: &SourceGameIdentity,
        authority: &str,
        layers: &[RecordedContentLayer],
    ) -> Result<(), String> {
        let Some(query) = self.complete() else {
            return Ok(());
        };
        if query.specification != NativeGraphicsQuerySpec::current()
            || source.executable_sha256 != NATIVE_GRAPHICS_EXECUTABLE_SHA256
            || query.authority_manifest_sha256 != authority
        {
            return Err("native_graphics_query_unknown_law_or_build".into());
        }
        if query.entries.len() > MAX_QUERY_ENTRIES
            || query.layers.len() != layers.len()
            || layers.is_empty()
            || layers.len() > MAX_QUERY_ENTRIES
            || layers[0].ordinal != 0
            || layers[0].layer_kind != ContentLayerKind::Base
        {
            return Err("native_graphics_query_scope_or_entry_bound".into());
        }
        for (observed, layer) in query.layers.iter().zip(layers) {
            if observed.ordinal != layer.ordinal
                || observed.layer_kind != layer.layer_kind
                || observed.layer_id != layer.layer_id
                || observed.storage_kind != layer.storage_kind
                || observed.source_container_sha256 != layer.source_container_sha256
                || observed.replace_paths != layer.replace_paths
                || !layer.required_for_playback
            {
                return Err("native_graphics_query_layer_scope_mismatch".into());
            }
        }
        crate::hud_role::validate_sha256(&query.scope_sha256, "native query scope")?;
        let mut previous: Option<&str> = None;
        for entry in &query.entries {
            let asked = &entry.original_query_spelling;
            let normalized = crate::normalize_game_relative_path(asked)?;
            crate::validate_stored_game_relative_path(asked, &normalized)?;
            if !asked.starts_with("interface/")
                || !asked.ends_with(".gfx")
                || asked.len() > MAX_QUERY_PATH_BYTES
                || asked.split('/').count() > MAX_QUERY_DEPTH + 2
                || normalized != entry.content.normalized_game_relative_path
                || previous.is_some_and(|p| native_query_path_cmp(p, asked).is_ge())
            {
                return Err("native_graphics_query_invalid_spelling_or_order".into());
            }
            previous = Some(asked);
            entry.content.validate()?;
            let layer = layers
                .iter()
                .find(|l| l.ordinal == entry.content.winning_layer_ordinal)
                .ok_or("native_graphics_query_missing_payload_layer")?;
            if entry.content.effective_vfs_manifest_sha256 != authority
                || entry.content.layer_kind != layer.layer_kind
                || entry.content.layer_id != layer.layer_id
                || layer
                    .source_container_sha256
                    .as_ref()
                    .unwrap_or(&entry.content.payload_sha256)
                    != &entry.content.source_sha256
                || (layer.storage_kind == LayerStorageKind::ZipArchive
                    && asked != &entry.content.stored_game_relative_path)
            {
                return Err("native_graphics_query_payload_identity_mismatch".into());
            }
        }
        self.required_content()
            .iter()
            .try_fold(0u64, |total, key| {
                total
                    .checked_add(key.payload_bytes)
                    .filter(|n| *n <= MAX_QUERY_PAYLOAD_BYTES)
                    .ok_or("native_graphics_query_payload_bound")
            })?;
        if query.digest()? != query.sequence_sha256 {
            return Err("native_graphics_query_sequence_digest_mismatch".into());
        }
        Ok(())
    }
    pub(crate) fn validate_playback(&self, actual: &[UsedContentKey]) -> Result<(), String> {
        let expected = self.required_content();
        if actual.len() != expected.len()
            || actual.iter().collect::<BTreeSet<_>>().len() != actual.len()
            || actual.iter().collect::<BTreeSet<_>>() != expected.iter().collect::<BTreeSet<_>>()
        {
            return Err("native_graphics_query_playback_payload_mismatch".into());
        }
        Ok(())
    }
}

/// Files in a directory precede every child directory, regardless of basename.
/// Directory recursion then follows the original byte spelling.
pub(crate) fn native_query_path_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a = a.split('/').peekable();
    let mut b = b.split('/').peekable();
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) => {
                let leaf_order = a.peek().is_some().cmp(&b.peek().is_some());
                if leaf_order.is_ne() {
                    return leaf_order;
                }
                let name_order = x.as_bytes().cmp(y.as_bytes());
                if name_order.is_ne() {
                    return name_order;
                }
            }
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
        }
    }
}

pub(crate) fn bounded_digest(value: &impl Serialize) -> Result<String, String> {
    struct Sink {
        hash: Sha256,
        bytes: u64,
    }
    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes = self
                .bytes
                .checked_add(bytes.len() as u64)
                .filter(|n| *n <= MAX_QUERY_BYTES)
                .ok_or_else(|| std::io::Error::other("native_graphics_query_metadata_bound"))?;
            self.hash.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink {
        hash: Sha256::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut sink, value).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", sink.hash.finalize()))
}

trait QueryWireSize {
    fn bounded_wire_size(&self) -> Option<u64>;
}
impl QueryWireSize for String {
    fn bounded_wire_size(&self) -> Option<u64> {
        (self.len() <= MAX_QUERY_PATH_BYTES).then_some(self.len() as u64 * 6 + 64)
    }
}
impl QueryWireSize for NativeGraphicsQueryEntry {
    fn bounded_wire_size(&self) -> Option<u64> {
        [
            &self.original_query_spelling,
            &self.content.stored_game_relative_path,
            &self.content.normalized_game_relative_path,
            &self.content.layer_id,
            &self.content.effective_vfs_manifest_sha256,
            &self.content.source_sha256,
            &self.content.payload_sha256,
        ]
        .into_iter()
        .try_fold(512u64, |n, s| n.checked_add(s.bounded_wire_size()?))
    }
}
impl QueryWireSize for NativeGraphicsQueryLayer {
    fn bounded_wire_size(&self) -> Option<u64> {
        let initial = 512 + self.layer_id.bounded_wire_size()?;
        self.replace_paths
            .iter()
            .try_fold(initial, |n, s| n.checked_add(s.bounded_wire_size()?))
    }
}
fn bounded_sequence<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + QueryWireSize,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de> + QueryWireSize> serde::de::Visitor<'de> for Visitor<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded native query rows")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            use serde::de::Error;
            if seq.size_hint().is_some_and(|n| n > MAX_QUERY_ENTRIES) {
                return Err(A::Error::custom("native_graphics_query_entry_bound"));
            }
            let mut rows = Vec::new();
            let mut bytes = 0u64;
            loop {
                if rows.len() == MAX_QUERY_ENTRIES {
                    if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                        return Err(A::Error::custom("native_graphics_query_entry_bound"));
                    }
                    break;
                }
                let Some(row) = seq.next_element::<T>()? else {
                    break;
                };
                bytes = row
                    .bounded_wire_size()
                    .and_then(|n| bytes.checked_add(n))
                    .filter(|n| *n <= MAX_QUERY_BYTES)
                    .ok_or_else(|| A::Error::custom("native_graphics_query_metadata_bound"))?;
                rows.push(row);
            }
            Ok(rows)
        }
    }
    deserializer.deserialize_seq(Visitor(std::marker::PhantomData))
}

pub struct ObservedNativeGraphicsQuery {
    pub(crate) witness: NativeGraphicsQueryWitness,
    pub(crate) executable_sha256: String,
}
pub struct SealedNativeGraphicsQuery {
    witness: NativeGraphicsQueryWitness,
}
impl ObservedNativeGraphicsQuery {
    pub fn witness(&self) -> &NativeGraphicsQueryWitness {
        &self.witness
    }

    pub fn seal(
        self,
        after: &crate::effective_vfs::EffectiveVfs,
    ) -> Result<SealedNativeGraphicsQuery, String> {
        let next = after.observe_native_graphics_query(&self.executable_sha256)?;
        let witness = if self.witness == next.witness {
            self.witness
        } else {
            NativeGraphicsQueryWitness::unavailable(
                NativeGraphicsQueryUnavailable::ChangedDuringObservation,
            )
        };
        Ok(SealedNativeGraphicsQuery { witness })
    }
}
impl SealedNativeGraphicsQuery {
    pub fn witness(&self) -> &NativeGraphicsQueryWitness {
        &self.witness
    }
    pub fn into_witness(self) -> NativeGraphicsQueryWitness {
        self.witness
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceGameIdentity {
        SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: NATIVE_GRAPHICS_EXECUTABLE_SHA256.into(),
            build_profile: "a729".into(),
        }
    }
    fn layers() -> Vec<RecordedContentLayer> {
        (0..2)
            .map(|ordinal| RecordedContentLayer {
                ordinal,
                layer_kind: if ordinal == 0 {
                    ContentLayerKind::Base
                } else {
                    ContentLayerKind::Mod
                },
                layer_id: format!("layer-{ordinal}"),
                display_name: format!("Layer {ordinal}"),
                descriptor_sha256: (ordinal != 0).then(|| "a".repeat(64)),
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            })
            .collect()
    }
    fn key(path: &str, ordinal: u32, payload: char) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "f".repeat(64),
            stored_game_relative_path: path.into(),
            normalized_game_relative_path: path.to_ascii_lowercase(),
            winning_layer_ordinal: ordinal,
            layer_kind: if ordinal == 0 {
                ContentLayerKind::Base
            } else {
                ContentLayerKind::Mod
            },
            layer_id: format!("layer-{ordinal}"),
            source_sha256: payload.to_string().repeat(64),
            payload_sha256: payload.to_string().repeat(64),
            payload_bytes: 3,
        }
    }
    // Protocol fixtures exercise identity capacity, not a claim that these rows
    // were observed from native code. Playback must separately reobserve scope.
    fn wire_fixture(entries: Vec<NativeGraphicsQueryEntry>) -> NativeGraphicsQueryWitness {
        NativeGraphicsQueryWitness::from_observation(
            "f".repeat(64),
            layers()
                .iter()
                .map(|l| NativeGraphicsQueryLayer {
                    ordinal: l.ordinal,
                    layer_kind: l.layer_kind,
                    layer_id: l.layer_id.clone(),
                    storage_kind: l.storage_kind,
                    source_container_sha256: l.source_container_sha256.clone(),
                    replace_paths: l.replace_paths.clone(),
                })
                .collect(),
            "e".repeat(64),
            entries,
        )
        .unwrap()
    }
    fn validate(witness: &NativeGraphicsQueryWitness) -> Result<(), String> {
        witness.validate(&source(), &"f".repeat(64), &layers())
    }

    #[test]
    fn wire_preserves_distinct_payloads_sharing_a_folded_path() {
        let upper = key("interface/A.gfx", 1, 'b');
        let lower = key("interface/a.gfx", 0, 'c');
        let witness = wire_fixture(vec![
            NativeGraphicsQueryEntry {
                original_query_spelling: "interface/A.gfx".into(),
                content: upper.clone(),
            },
            NativeGraphicsQueryEntry {
                original_query_spelling: "interface/a.gfx".into(),
                content: lower.clone(),
            },
        ]);
        validate(&witness).unwrap();
        assert_eq!(
            upper.normalized_game_relative_path,
            lower.normalized_game_relative_path
        );
        assert_ne!(upper, lower);
        witness
            .validate_playback(&[upper.clone(), lower.clone()])
            .unwrap();
        assert!(witness.validate_playback(&[upper.clone()]).is_err());
        assert!(witness.validate_playback(&[upper.clone(), upper]).is_err());
        let bytes = serde_json::to_vec(&witness).unwrap();
        let decoded: NativeGraphicsQueryWitness = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(witness, decoded);
        assert_eq!(decoded.required_content().len(), 2);
    }

    #[test]
    fn wire_preserves_multiple_spellings_of_one_payload() {
        let content = key("interface/A.gfx", 1, 'b');
        let witness = wire_fixture(
            ["interface/A.gfx", "interface/a.gfx"]
                .into_iter()
                .map(|asked| NativeGraphicsQueryEntry {
                    original_query_spelling: asked.into(),
                    content: content.clone(),
                })
                .collect(),
        );
        validate(&witness).unwrap();
        assert_eq!(witness.complete().unwrap().entries().len(), 2);
        assert_eq!(witness.required_content(), [content.clone()]);
        witness.validate_playback(&[content]).unwrap();
    }

    #[test]
    fn witness_requires_known_law_and_complete_scope_even_without_payloads() {
        let witness = wire_fixture(Vec::new());
        validate(&witness).unwrap();
        assert_eq!(
            witness.required_layer_ordinals().collect::<Vec<_>>(),
            [0, 1]
        );
        let mut optional = layers();
        optional[1].required_for_playback = false;
        assert!(witness
            .validate(&source(), &"f".repeat(64), &optional)
            .is_err());
        assert!(witness
            .validate(&source(), &"0".repeat(64), &layers())
            .is_err());
        for changed_field in ["law", "executable_sha256", "directory", "suffix", "depth"] {
            let mut json = serde_json::to_value(&witness).unwrap();
            json["query"]["specification"][changed_field] = if changed_field == "depth" {
                serde_json::json!(1)
            } else {
                serde_json::json!("unknown")
            };
            let changed: NativeGraphicsQueryWitness = serde_json::from_value(json).unwrap();
            assert!(validate(&changed).is_err(), "{changed_field}");
        }
        for missing in ["contains", "law", "suffix"] {
            let mut json = serde_json::to_value(&witness).unwrap();
            json["query"]["specification"]
                .as_object_mut()
                .unwrap()
                .remove(missing);
            assert!(
                serde_json::from_value::<NativeGraphicsQueryWitness>(json).is_err(),
                "{missing}"
            );
        }
        assert!(NativeGraphicsQueryWitness::default().complete().is_none());
        assert_eq!(
            NativeGraphicsQueryWitness::default().unavailable_cause(),
            Some(&NativeGraphicsQueryUnavailable::NotObserved)
        );
        assert!(
            serde_json::from_str::<NativeGraphicsQueryWitness>("{\"status\":\"unknown\"}").is_err()
        );
    }

    #[test]
    fn rows_must_have_native_order_and_digest_bound_identity() {
        let paths = [
            "interface/Z.gfx",
            "interface/a/first.gfx",
            "interface/a/sub/deep.gfx",
            "interface/z/end.gfx",
        ];
        let entries = paths
            .into_iter()
            .map(|p| NativeGraphicsQueryEntry {
                original_query_spelling: p.into(),
                content: key(p, 0, 'a'),
            })
            .collect();
        let witness = wire_fixture(entries);
        validate(&witness).unwrap();
        let mut json = serde_json::to_value(&witness).unwrap();
        json["query"]["entries"].as_array_mut().unwrap().swap(0, 1);
        let changed: NativeGraphicsQueryWitness = serde_json::from_value(json).unwrap();
        assert!(validate(&changed).is_err());
        let mut json = serde_json::to_value(&witness).unwrap();
        json["query"]["scope_sha256"] = serde_json::json!("d".repeat(64));
        let changed: NativeGraphicsQueryWitness = serde_json::from_value(json).unwrap();
        assert!(validate(&changed).unwrap_err().contains("digest"));
        let duplicate = wire_fixture(vec![
            NativeGraphicsQueryEntry {
                original_query_spelling: paths[0].into(),
                content: key(paths[0], 0, 'a'),
            },
            NativeGraphicsQueryEntry {
                original_query_spelling: paths[0].into(),
                content: key(paths[0], 0, 'a'),
            },
        ]);
        assert!(validate(&duplicate).is_err());
    }

    #[test]
    fn query_payload_and_wire_bounds_are_validated() {
        let mut content = key("interface/a.gfx", 0, 'a');
        content.payload_bytes = crate::MAX_CONTENT_PAYLOAD_BYTES + 1;
        let witness = wire_fixture(vec![NativeGraphicsQueryEntry {
            original_query_spelling: "interface/a.gfx".into(),
            content,
        }]);
        assert!(validate(&witness).is_err());
        let mut json = serde_json::to_value(wire_fixture(Vec::new())).unwrap();
        json["query"]["layers"][0]["layer_id"] =
            serde_json::json!("x".repeat(MAX_QUERY_PATH_BYTES + 1));
        assert!(serde_json::from_value::<NativeGraphicsQueryWitness>(json).is_err());
        let entries = (0..33)
            .map(|i| {
                let path = format!("interface/{i:02}.gfx");
                let mut content = key(&path, 0, 'a');
                content.payload_bytes = crate::MAX_CONTENT_PAYLOAD_BYTES;
                NativeGraphicsQueryEntry {
                    original_query_spelling: path,
                    content,
                }
            })
            .collect::<Vec<_>>();
        assert!(validate(&wire_fixture(entries[..32].to_vec())).is_ok());
        assert!(validate(&wire_fixture(entries))
            .unwrap_err()
            .contains("payload_bound"));
    }
}
