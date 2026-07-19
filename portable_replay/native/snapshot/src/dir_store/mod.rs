//! Anchored, handle-relative directory-store access.
//!
//! The root pathname is resolved exactly once. Every descendant component is
//! then opened relative to its already-open parent, checked on that object, and
//! read from that handle. No descendant pathname is retained or re-resolved.

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(all(unix, not(target_os = "linux")))]
mod other_posix;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(all(unix, not(target_os = "linux")))]
use other_posix as platform;
#[cfg(windows)]
use windows as platform;

#[derive(Debug, Clone)]
pub(crate) struct DirAccessError {
    pub(crate) not_found: bool,
    pub(crate) message: String,
}

impl DirAccessError {
    pub(crate) fn reject(message: impl Into<String>) -> Self {
        Self {
            not_found: false,
            message: message.into(),
        }
    }
    pub(crate) fn missing(message: impl Into<String>) -> Self {
        Self {
            not_found: true,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DirAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DirAccessError {}

#[derive(Debug, Clone)]
pub(crate) struct RootEntry {
    pub(crate) name: String,
    pub(crate) directory_or_link: bool,
}

#[derive(Clone)]
pub(crate) struct AnchoredDir {
    root: Arc<platform::PlatformRoot>,
    label: Arc<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct BoundDir {
    parent: AnchoredDir,
    child: AnchoredDir,
    component: Arc<str>,
}

impl std::fmt::Debug for AnchoredDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnchoredDir")
            .field("root_label", &self.label)
            .finish_non_exhaustive()
    }
}

impl AnchoredDir {
    pub(crate) fn open(path: &Path) -> Result<Self, DirAccessError> {
        Ok(Self {
            root: Arc::new(platform::PlatformRoot::open(path)?),
            label: Arc::new(path.to_path_buf()),
        })
    }

    pub(crate) fn bind_child_dir(&self, component: &str) -> Result<BoundDir, DirAccessError> {
        validate_components(&[component])?;
        let child = Self {
            root: Arc::new(self.root.open_child_dir(component)?),
            label: self.label.clone(),
        };
        Ok(BoundDir {
            parent: self.clone(),
            child,
            component: Arc::from(component),
        })
    }

    pub(crate) fn root_entries(&self) -> Result<Vec<RootEntry>, DirAccessError> {
        self.root.root_entries()
    }

    pub(crate) fn read_file(&self, components: &[&str]) -> Result<Vec<u8>, DirAccessError> {
        validate_components(components)?;
        self.root.read_file(components)
    }
}

impl std::fmt::Debug for BoundDir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundDir")
            .field("component", &self.component)
            .finish_non_exhaustive()
    }
}

impl BoundDir {
    pub(crate) fn revalidate(&self) -> Result<(), DirAccessError> {
        self.parent
            .root
            .validate_child_dir(&self.component, &self.child.root)
    }

    pub(crate) fn read_file(&self, components: &[&str]) -> Result<Vec<u8>, DirAccessError> {
        self.revalidate()?;
        self.child.read_file(components)
    }

    pub(crate) fn read_optional(
        &self,
        components: &[&str],
    ) -> Result<Option<Vec<u8>>, DirAccessError> {
        match self.read_file(components) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.not_found => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn validate_components(components: &[&str]) -> Result<(), DirAccessError> {
    if components.is_empty() {
        return Err(DirAccessError::reject("relative walk has no components"));
    }
    for component in components {
        if component.is_empty()
            || *component == "."
            || *component == ".."
            || component.contains('/')
            || component.contains(char::from(92))
            || component.contains(':')
        {
            return Err(DirAccessError::reject(format!(
                "unsafe relative component {component:?}"
            )));
        }
    }
    Ok(())
}

pub(super) const NAME_SURROGATE_BIT: u32 = 0x2000_0000;

pub(super) fn is_name_surrogate(tag: u32) -> bool {
    tag & NAME_SURROGATE_BIT != 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReparseRead {
    Inspected,
    Reopen,
}

pub(super) fn reparse_read(tag: u32) -> Result<ReparseRead, DirAccessError> {
    if is_name_surrogate(tag) {
        Err(DirAccessError::reject(format!(
            "name-surrogate reparse tag {tag:#010x} rejected"
        )))
    } else if tag == 0 {
        Ok(ReparseRead::Inspected)
    } else {
        Ok(ReparseRead::Reopen)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObjectId {
    pub(super) volume: u64,
    pub(super) file: [u8; 16],
}

impl ObjectId {
    pub(super) fn from_windows_file_id(
        volume: u64,
        file: [u8; 16],
    ) -> Result<Self, DirAccessError> {
        let sentinel = if file.iter().all(|byte| *byte == 0) {
            Some("all-zero")
        } else if file.iter().all(|byte| *byte == u8::MAX) {
            Some("all-ones")
        } else {
            None
        };
        if let Some(sentinel) = sentinel {
            return Err(DirAccessError::reject(format!(
                "Windows 128-bit file identity unavailable: {sentinel} sentinel"
            )));
        }
        Ok(Self { volume, file })
    }

    fn windows_file_id_is_unavailable(&self) -> bool {
        self.file.iter().all(|byte| *byte == 0) || self.file.iter().all(|byte| *byte == u8::MAX)
    }
}

#[derive(Debug, Clone)]
pub(super) struct ListedObject {
    pub(super) name: String,
    pub(super) id: ObjectId,
}

/// Bind the requested component to the opened object and to exactly one name
/// in the same parent. This is also the directory lookalike policy gate.
pub(super) fn validate_binding(
    expected: &str,
    actual_name: &str,
    opened_id: &ObjectId,
    entries: &[ListedObject],
) -> Result<(), DirAccessError> {
    let lookalikes: Vec<&str> = entries
        .iter()
        .map(|entry| entry.name.as_str())
        .filter(|name| name.eq_ignore_ascii_case(expected))
        .collect();
    if lookalikes.iter().any(|name| *name != expected) {
        return Err(DirAccessError::reject(format!(
            "ASCII-lookalike policy rejected component {expected:?}; variants: {lookalikes:?}"
        )));
    }
    if actual_name != expected {
        return Err(DirAccessError::reject(format!(
            "namespace binding actual-name check for {expected:?} opened stored name {actual_name:?}"
        )));
    }
    let aliases: Vec<&ListedObject> = entries
        .iter()
        .filter(|entry| entry.id == *opened_id)
        .collect();
    match aliases.as_slice() {
        [only] if only.name == expected => Ok(()),
        [] => Err(DirAccessError::reject(format!(
            "namespace binding identity check could not find opened {expected:?} in its parent"
        ))),
        [_] => Err(DirAccessError::reject(format!(
            "namespace binding stored-name check for {expected:?} disagrees with parent enumeration"
        ))),
        _ => Err(DirAccessError::reject(format!(
            "same-parent identity alias is ambiguous for {expected:?}: {:?}",
            aliases
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
        ))),
    }
}

pub(super) fn validate_reopen_identity(
    inspected: &ObjectId,
    reopened: &ObjectId,
) -> Result<(), DirAccessError> {
    if inspected.windows_file_id_is_unavailable() {
        return Err(DirAccessError::reject(
            "identity unavailable: inspected Windows 128-bit file ID is a sentinel",
        ));
    }
    if reopened.windows_file_id_is_unavailable() {
        return Err(DirAccessError::reject(
            "identity unavailable: reopened Windows 128-bit file ID is a sentinel",
        ));
    }
    if inspected == reopened {
        Ok(())
    } else {
        Err(DirAccessError::reject(format!(
            "identity-bound reopen mismatch: inspected {inspected:?}, reopened {reopened:?}"
        )))
    }
}

/// Select the only handle from which bytes may be read. Production and the
/// tag-injection tests share this function so a permitted non-surrogate tag
/// cannot bypass the reopen-and-identity check without breaking the gate.
pub(super) fn identity_bound_handle<T, F>(
    tag: u32,
    inspected: T,
    inspected_id: &ObjectId,
    reopen: F,
) -> Result<(T, ObjectId), DirAccessError>
where
    F: FnOnce() -> Result<(T, ObjectId), DirAccessError>,
{
    match reparse_read(tag)? {
        ReparseRead::Inspected => Ok((inspected, inspected_id.clone())),
        ReparseRead::Reopen => {
            let (reopened, reopened_id) = reopen()?;
            validate_reopen_identity(inspected_id, &reopened_id)?;
            Ok((reopened, reopened_id))
        }
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn validate_mount_instance(
    parent_mount_id: u64,
    child_mount_id: u64,
) -> Result<(), DirAccessError> {
    if parent_mount_id == child_mount_id {
        Ok(())
    } else {
        Err(DirAccessError::reject(format!(
            "mount-instance containment rejected crossing {parent_mount_id} -> {child_mount_id}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ObjectId {
        ObjectId {
            volume: 7,
            file: [n; 16],
        }
    }

    #[test]
    fn surrogate_discriminator_is_the_normative_bit() {
        for tag in [0xA000_000C, 0xA000_0003, 0x2000_1234] {
            assert!(is_name_surrogate(tag), "{tag:#010x}");
        }
        for tag in [0x9000_001A, 0x8000_0017, 0x0000_1234] {
            assert!(!is_name_surrogate(tag), "{tag:#010x}");
        }
    }

    #[test]
    fn binding_check_is_non_vacuous_and_aliases_fail_closed() {
        let canonical = ListedObject {
            name: "timeline.json".into(),
            id: id(1),
        };
        let error = validate_binding(
            "timeline.json",
            "Timeline.json",
            &id(1),
            std::slice::from_ref(&canonical),
        )
        .unwrap_err();
        assert!(error.message.contains("namespace binding actual-name"));

        let alias = ListedObject {
            name: "backup.json".into(),
            id: id(1),
        };
        let error = validate_binding(
            "timeline.json",
            "timeline.json",
            &id(1),
            &[canonical, alias],
        )
        .unwrap_err();
        assert!(error.message.contains("same-parent identity alias"));
    }

    #[test]
    fn intermediate_multi_variant_policy_rejects_even_with_canonical_present() {
        let entries = [
            ListedObject {
                name: "recorder".into(),
                id: id(1),
            },
            ListedObject {
                name: "Recorder".into(),
                id: id(2),
            },
        ];
        let error = validate_binding("recorder", "recorder", &id(1), &entries).unwrap_err();
        assert!(error.message.contains("ASCII-lookalike policy"), "{error}");
        assert!(error.message.contains("Recorder"), "{error}");
    }

    #[test]
    fn mount_id_not_device_number_controls_containment() {
        let same_device = (7u64, 7u64);
        assert_eq!(same_device.0, same_device.1);
        assert!(validate_mount_instance(11, 12)
            .unwrap_err()
            .message
            .contains("mount-instance"));
        let overlay_shape = (7u64, 9u64);
        assert_ne!(overlay_shape.0, overlay_shape.1);
        assert!(validate_mount_instance(11, 11).is_ok());
    }

    #[test]
    fn reopen_identity_mismatch_is_attributed() {
        let error = validate_reopen_identity(&id(1), &id(2)).unwrap_err();
        assert!(error.message.contains("identity-bound reopen"));
    }

    #[test]
    fn windows_unavailable_file_id_sentinels_cannot_bind_a_reopen() {
        for file in [[0u8; 16], [u8::MAX; 16]] {
            let unavailable = ObjectId { volume: 7, file };
            let error = validate_reopen_identity(&unavailable, &unavailable)
                .expect_err("an unavailable Windows file ID cannot establish identity");
            assert!(error.message.contains("identity unavailable"), "{error}");
        }
    }

    #[test]
    fn cloud_placeholder_double_reads_only_from_identity_bound_reopen() {
        let cloud_tag = 0x9000_001A;
        let mut trace = vec!["inspect"];
        let (selected, _) =
            identity_bound_handle(cloud_tag, "inspected placeholder bytes", &id(1), || {
                trace.push("reopen");
                Ok(("logical hydrated bytes", id(1)))
            })
            .unwrap();
        trace.push("read reopened");
        assert_eq!(trace, ["inspect", "reopen", "read reopened"]);
        assert_eq!(selected, "logical hydrated bytes");

        let mut plain_reopens = 0;
        let (selected, _) = identity_bound_handle(0, "plain bytes", &id(2), || {
            plain_reopens += 1;
            Ok(("wrong reopened bytes", id(2)))
        })
        .unwrap();
        assert_eq!(plain_reopens, 0, "plain files permit exactly one open");
        assert_eq!(selected, "plain bytes");
    }

    #[test]
    fn genuine_legacy_timeline_survives_simulated_placeholder_tag() {
        let inspected = id(3);
        let logical_bytes = match reparse_read(0x9000_001A).unwrap() {
            ReparseRead::Inspected => panic!("cloud tag must force identity-bound reopen"),
            ReparseRead::Reopen => {
                validate_reopen_identity(&inspected, &id(3)).unwrap();
                br#"{"legacy":true}"#.as_slice()
            }
        };
        let timeline = crate::parse_json_strict(logical_bytes).unwrap();
        assert!(timeline.get("session_manifest").is_none());
        assert_eq!(timeline["legacy"], true);
    }
}
