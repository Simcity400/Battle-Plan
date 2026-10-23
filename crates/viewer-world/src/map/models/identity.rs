use installed_content_core::{ResolvedLayers, content::LayerSource};
use installed_content_image::native::Decode;
use std::{
    cmp::Ordering,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Default)]
pub struct AssetKey(Option<Arc<Identity>>);

#[derive(Debug)]
struct Identity {
    entity: Arc<str>,
    namespace: Arc<[ContentLayer]>,
    role: Role,
    contract: (u32, bool),
    fingerprint: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ContentLayer {
    id: String,
    source: PathBuf,
    archive: bool,
    replace_paths: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Role {
    Selected,
    RecordedRetry,
    RecordedCounterpart,
}

#[derive(Default)]
struct Namespaces(Mutex<Vec<Arc<[ContentLayer]>>>);

/// A live pass namespace, effective asset and unchanged producer shared-player key.
pub type ActorAddress = (u64, AssetKey, u32, u32);

pub(super) fn namespace(layers: &ResolvedLayers) -> Arc<[ContentLayer]> {
    namespace_for(layers, layers)
}

pub(super) fn namespace_for(
    layers: &ResolvedLayers,
    session: &ResolvedLayers,
) -> Arc<[ContentLayer]> {
    let cache = session.derived::<Namespaces>();
    let mut namespaces = cache.0.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(known) = namespaces.iter().find(|known| {
        known.len() == layers.layers.len()
            && known.iter().zip(&layers.layers).all(|(known, layer)| {
                let (source, archive) = match &layer.source {
                    LayerSource::Folder(path) => (path, false),
                    LayerSource::Archive(path) => (path, true),
                };
                known.id == layer.id
                    && &known.source == source
                    && known.archive == archive
                    && known.replace_paths == layer.replace_paths
            })
    }) {
        return Arc::clone(known);
    }
    let namespace: Arc<[ContentLayer]> = layers
        .layers
        .iter()
        .map(|layer| {
            let (source, archive) = match &layer.source {
                LayerSource::Folder(path) => (path.clone(), false),
                LayerSource::Archive(path) => (path.clone(), true),
            };
            ContentLayer {
                id: layer.id.clone(),
                source,
                archive,
                replace_paths: layer.replace_paths.clone(),
            }
        })
        .collect();
    namespaces.push(Arc::clone(&namespace));
    namespace
}

impl Identity {
    fn key(
        entity: Arc<str>,
        namespace: Arc<[ContentLayer]>,
        role: Role,
        contract: (u32, bool),
    ) -> AssetKey {
        let mut hash = DefaultHasher::new();
        (&entity, &namespace, role, contract).hash(&mut hash);
        AssetKey(Some(Arc::new(Self {
            entity,
            namespace,
            role,
            contract,
            fingerprint: hash.finish(),
        })))
    }

    fn compare(&self, other: &Self) -> Ordering {
        self.entity
            .cmp(&other.entity)
            .then_with(|| {
                if Arc::ptr_eq(&self.namespace, &other.namespace) {
                    Ordering::Equal
                } else {
                    self.namespace.cmp(&other.namespace)
                }
            })
            .then_with(|| self.role.cmp(&other.role))
            .then_with(|| self.contract.cmp(&other.contract))
    }
}

impl PartialEq for AssetKey {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a.compare(b) == Ordering::Equal,
            (None, None) => true,
            _ => false,
        }
    }
}
impl Eq for AssetKey {}
impl PartialOrd for AssetKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for AssetKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) if Arc::ptr_eq(a, b) => Ordering::Equal,
            (Some(a), Some(b)) => a.compare(b),
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
        }
    }
}
impl Hash for AssetKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0
            .as_ref()
            .map_or(0, |identity| identity.fingerprint)
            .hash(state);
    }
}

impl AssetKey {
    pub(super) fn selected(name: &str, namespace: Arc<[ContentLayer]>, decode: Decode) -> Self {
        Identity::key(
            name.into(),
            namespace,
            Role::Selected,
            (1, decode.force_pow2_textures),
        )
    }

    pub(super) fn recorded(&mut self, counterpart: bool) {
        let identity = self.0.as_ref().expect("a baked asset has an identity");
        *self = Identity::key(
            Arc::clone(&identity.entity),
            Arc::clone(&identity.namespace),
            if counterpart {
                Role::RecordedCounterpart
            } else {
                Role::RecordedRetry
            },
            identity.contract,
        );
    }

    #[cfg(test)]
    pub(crate) fn synthetic(name: &str) -> Self {
        Self::selected(name, Arc::from([]), Decode::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn namespace_keeps_mount_order_source_kind_and_replace_paths() {
        let layer = |id: &str| ContentLayer {
            id: id.into(),
            source: PathBuf::from(id),
            archive: false,
            replace_paths: Vec::new(),
        };
        let a = layer("one");
        let b = layer("two");
        let key = |layers: Vec<ContentLayer>| {
            AssetKey::selected("same", layers.into(), Decode::default())
        };
        let ordinary = key(vec![a.clone(), b.clone()]);
        let reversed = key(vec![b.clone(), a.clone()]);
        let mut archive = a.clone();
        archive.archive = true;
        let mut replaced = a.clone();
        replaced.replace_paths.push(b"gfx/models".to_vec());
        let keys = BTreeSet::from([
            ordinary.clone(),
            reversed,
            key(vec![archive, b.clone()]),
            key(vec![replaced, b.clone()]),
        ]);
        assert_eq!(keys.len(), 4);
        assert_eq!(ordinary, key(vec![a, b]));
    }

    fn hashed(key: &AssetKey) -> u64 {
        let mut hash = DefaultHasher::new();
        key.hash(&mut hash);
        hash.finish()
    }

    #[test]
    fn independently_allocated_equal_keys_hash_and_order_identically() {
        let layer = || ContentLayer {
            id: "mod/session".into(),
            source: PathBuf::from("synthetic/mod"),
            archive: false,
            replace_paths: vec![b"gfx/models".to_vec()],
        };
        let mut a = AssetKey::selected("same", vec![layer()].into(), Decode::default());
        let mut b = AssetKey::selected("same", vec![layer()].into(), Decode::default());
        assert!(!Arc::ptr_eq(a.0.as_ref().unwrap(), b.0.as_ref().unwrap()));
        assert_eq!(a, b);
        assert_eq!(a.cmp(&b), Ordering::Equal);
        assert_eq!(hashed(&a), hashed(&b));
        let selected_hash = hashed(&a);
        for counterpart in [false, true] {
            a.recorded(counterpart);
            b.recorded(counterpart);
            assert_eq!(a, b);
            assert_eq!(a.cmp(&b), Ordering::Equal);
            assert_eq!(hashed(&a), hashed(&b));
            assert_ne!(selected_hash, hashed(&a));
        }
    }

    #[test]
    fn fingerprint_collisions_never_decide_identity_or_lexicographic_order() {
        let a = AssetKey::synthetic("alpha");
        let mut b = AssetKey::synthetic("zeta");
        Arc::get_mut(b.0.as_mut().unwrap()).unwrap().fingerprint =
            a.0.as_ref().unwrap().fingerprint;
        assert_eq!(hashed(&a), hashed(&b));
        assert_ne!(a, b);
        assert_eq!(a.cmp(&b), Ordering::Less);
        assert_eq!(b.cmp(&a), Ordering::Greater);
        assert_eq!(BTreeSet::from([b, a]).len(), 2);
    }

    #[test]
    fn content_session_and_its_shards_share_the_namespace_allocation() {
        let stack = installed_content_core::test_support::Stack::new("model_namespace");
        let layers = stack.layers(&[]);
        let cloned = layers.clone();
        let a = namespace(&layers);
        let b = std::thread::scope(|scope| scope.spawn(|| namespace(&cloned)).join().unwrap());
        assert!(Arc::ptr_eq(&a, &b));
        assert!(AssetKey::default().0.is_none());
    }

    #[test]
    fn session_namespace_cache_keeps_the_full_value_when_a_mount_is_edited() {
        let stack = installed_content_core::test_support::Stack::new("model_namespace_edit");
        let layers = stack.layers(&[]);
        let original = namespace(&layers);
        let mut changed = layers.clone();
        changed.layers[0].replace_paths.push(b"gfx/models".to_vec());
        let replaced = namespace(&changed);
        assert!(!Arc::ptr_eq(&original, &replaced));
        assert_ne!(
            AssetKey::selected("same", original.clone(), Decode::default()),
            AssetKey::selected("same", replaced, Decode::default())
        );
        assert!(Arc::ptr_eq(&original, &namespace(&layers)));
    }
}
