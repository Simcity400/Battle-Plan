use crate::effective_vfs::{EffectiveVfs, EffectiveVfsEntryFacts};
use crate::{
    ContentLayerKind, ContentManifest, RecordedContentLayer, RequiredContent, UsedContentKey,
};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;

pub const MAX_NAMESPACE_CERTIFICATES: usize = 64;
pub const MAX_NAMESPACE_WINNERS: usize = 400_000;
pub const MAX_NAMESPACE_REFERENCES: usize = 1_048_576;
pub const MAX_NAMESPACE_ROSTER_BYTES: u64 = 512 << 20;
pub const MAX_NAMESPACE_PAYLOAD_BYTES: u64 = 64 << 30;

#[derive(Clone, Copy, Debug)]
pub struct ContentNamespaceLimits {
    pub winners: usize,
    pub metadata_entries: usize,
    pub roster_bytes: u64,
    pub payload_bytes: u64,
}

impl Default for ContentNamespaceLimits {
    fn default() -> Self {
        Self {
            winners: MAX_NAMESPACE_WINNERS,
            metadata_entries: MAX_NAMESPACE_REFERENCES,
            roster_bytes: MAX_NAMESPACE_ROSTER_BYTES,
            payload_bytes: MAX_NAMESPACE_PAYLOAD_BYTES,
        }
    }
}

pub struct ContentNamespaceBudget {
    limits: ContentNamespaceLimits,
    winners: usize,
    payload_bytes: u64,
}

impl ContentNamespaceBudget {
    pub fn new(limits: ContentNamespaceLimits) -> Result<Self, String> {
        if limits.winners > MAX_NAMESPACE_WINNERS
            || limits.metadata_entries > MAX_NAMESPACE_REFERENCES
            || limits.roster_bytes > MAX_NAMESPACE_ROSTER_BYTES
            || limits.payload_bytes > MAX_NAMESPACE_PAYLOAD_BYTES
        {
            return Err("content_namespace_invalid_budget: exceeds production ceiling".into());
        }
        Ok(Self {
            limits,
            winners: 0,
            payload_bytes: 0,
        })
    }

    pub fn observed_winners(&self) -> usize {
        self.winners
    }
    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }
}

fn required_option<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentNamespaceScope {
    pub directory: String,
    #[serde(deserialize_with = "required_option")]
    pub extension: Option<String>,
    pub recursive: bool,
}

impl ContentNamespaceScope {
    pub fn root() -> Self {
        Self {
            directory: String::new(),
            extension: None,
            recursive: true,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if (!self.directory.is_empty()
            && crate::normalize_game_relative_path(&self.directory)? != self.directory)
            || self.directory.len() > 4096
            || self.extension.as_ref().is_some_and(|e| {
                e.is_empty()
                    || e.len() > 32
                    || e != &e.to_ascii_lowercase()
                    || e.chars()
                        .any(|c| c.is_control() || matches!(c, '/' | '\\' | '.'))
            })
        {
            return Err("content_namespace_invalid_scope".into());
        }
        Ok(())
    }

    pub fn matches(&self, path: &str) -> bool {
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        let under = self.directory.is_empty()
            || path
                .strip_prefix(&self.directory)
                .is_some_and(|suffix| suffix.starts_with('/'));
        under
            && (self.recursive || parent == self.directory)
            && self.extension.as_ref().is_none_or(|e| {
                path.rsplit_once('.')
                    .is_some_and(|(_, actual)| actual.eq_ignore_ascii_case(e))
            })
    }

    pub fn covers(&self, request: &Self) -> bool {
        let directory = if self.recursive {
            self.directory.is_empty()
                || self.directory == request.directory
                || request
                    .directory
                    .strip_prefix(&self.directory)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        } else {
            !request.recursive && self.directory == request.directory
        };
        directory && (self.extension.is_none() || self.extension == request.extension)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceExclusionCause {
    EngineWrittenInstallRootSink,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludedNamespaceWinner {
    pub stored_game_relative_path: String,
    pub normalized_game_relative_path: String,
    pub winning_layer_ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    pub cause: NamespaceExclusionCause,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentNamespaceCertificate {
    pub directory: String,
    #[serde(deserialize_with = "required_option")]
    pub extension: Option<String>,
    pub recursive: bool,
    pub winner_count: u32,
    pub winners_sha256: String,
    pub excluded_winners: Vec<ExcludedNamespaceWinner>,
}

impl ContentNamespaceCertificate {
    pub fn scope(&self) -> ContentNamespaceScope {
        ContentNamespaceScope {
            directory: self.directory.clone(),
            extension: self.extension.clone(),
            recursive: self.recursive,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentNamespaceRefusal {
    Unwitnessed {
        scope: ContentNamespaceScope,
    },
    ExcludedWinner {
        scope: ContentNamespaceScope,
        path: String,
        cause: NamespaceExclusionCause,
    },
    ContradictoryCertificate {
        scope: ContentNamespaceScope,
        witness: String,
    },
}

impl std::fmt::Display for ContentNamespaceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unwitnessed { scope } => write!(f, "content_namespace_unwitnessed: {scope:?}"),
            Self::ExcludedWinner { scope, path, cause } => write!(
                f,
                "content_namespace_excluded_winner: {scope:?}: {path}: {cause:?}"
            ),
            Self::ContradictoryCertificate { scope, witness } => write!(
                f,
                "content_namespace_contradictory_certificate: {scope:?}: {witness}"
            ),
        }
    }
}
impl std::error::Error for ContentNamespaceRefusal {}

struct DigestWriter {
    hash: Sha256,
    bytes: u64,
    limit: u64,
}
impl Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = self
            .bytes
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| {
                std::io::Error::other("content_namespace_bound_exceeded: canonical bytes")
            })?;
        self.hash.update(bytes);
        self.bytes = count;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn commitment(
    scope: &ContentNamespaceScope,
    authority: &str,
    excluded: &[ExcludedNamespaceWinner],
    rows: &[&UsedContentKey],
    limit: u64,
) -> Result<(String, u64), String> {
    let exclusions: Vec<_> = excluded
        .iter()
        .map(|e| {
            (
                &e.stored_game_relative_path,
                &e.normalized_game_relative_path,
                e.winning_layer_ordinal,
                &e.layer_kind,
                &e.layer_id,
                &e.cause,
            )
        })
        .collect();
    let winners: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                &r.effective_vfs_manifest_sha256,
                &r.stored_game_relative_path,
                &r.normalized_game_relative_path,
                r.winning_layer_ordinal,
                &r.layer_kind,
                &r.layer_id,
                &r.source_sha256,
                &r.payload_sha256,
                r.payload_bytes,
            )
        })
        .collect();
    let mut sink = DigestWriter {
        hash: Sha256::new(),
        bytes: 0,
        limit,
    };
    serde_json::to_writer(
        &mut sink,
        &(
            "hoi4.content-namespace-roster.v1",
            &scope.directory,
            &scope.extension,
            scope.recursive,
            authority,
            exclusions,
            winners,
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok((format!("{:x}", sink.hash.finalize()), sink.bytes))
}

fn ordered_rows(rows: &[&UsedContentKey]) -> Result<(), String> {
    if rows.len() > MAX_NAMESPACE_WINNERS {
        return Err("content_namespace_bound_exceeded: winner count".into());
    }
    if rows
        .windows(2)
        .any(|p| p[0].normalized_game_relative_path >= p[1].normalized_game_relative_path)
    {
        return Err("content_namespace_roster_order_or_duplicate".into());
    }
    Ok(())
}

pub(crate) fn validate_certificates(
    certificates: &[ContentNamespaceCertificate],
    required: &[RequiredContent],
    authority: &str,
    layers: &[RecordedContentLayer],
) -> Result<(), String> {
    if certificates.len() > MAX_NAMESPACE_CERTIFICATES {
        return Err("content_namespace_bound_exceeded: certificate count".into());
    }
    certificates.iter().try_fold(0usize, |total, cert| {
        if cert.winner_count as usize > MAX_NAMESPACE_WINNERS || cert.excluded_winners.len() > 3 {
            return Err("content_namespace_bound_exceeded: declared roster");
        }
        total
            .checked_add(cert.winner_count as usize)
            .filter(|n| *n <= MAX_NAMESPACE_REFERENCES)
            .ok_or("content_namespace_bound_exceeded: aggregate references")
    })?;
    let mut previous = None;
    let mut encoded_bytes = 0u64;
    for cert in certificates {
        let scope = cert.scope();
        scope.validate()?;
        if previous.as_ref().is_some_and(|p| p >= &scope) {
            return Err("content_namespace_certificate_order_or_duplicate".into());
        }
        previous = Some(scope.clone());
        crate::hud_role::validate_sha256(&cert.winners_sha256, "namespace winners")?;
        let rows: Vec<_> = required
            .iter()
            .map(|r| &r.content)
            .filter(|r| scope.matches(&r.normalized_game_relative_path))
            .collect();
        ordered_rows(&rows)?;
        if rows.len() != cert.winner_count as usize {
            return Err("content_namespace_winner_count_mismatch".into());
        }
        let mut prior: Option<&str> = None;
        for e in &cert.excluded_winners {
            let path = e.normalized_game_relative_path.as_str();
            if crate::normalize_game_relative_path(&e.stored_game_relative_path)? != path
                || !scope.matches(path)
                || prior.is_some_and(|p| p >= path)
                || !crate::is_engine_written_install_root_sink(path, e.layer_kind)
                || required
                    .iter()
                    .any(|r| r.content.normalized_game_relative_path == path)
                || !layers.iter().any(|l| {
                    l.ordinal == e.winning_layer_ordinal
                        && l.layer_kind == e.layer_kind
                        && l.layer_id == e.layer_id
                })
            {
                return Err("content_namespace_invalid_excluded_winner".into());
            }
            prior = Some(path);
        }
        let (digest, bytes) = commitment(
            &scope,
            authority,
            &cert.excluded_winners,
            &rows,
            MAX_NAMESPACE_ROSTER_BYTES - encoded_bytes,
        )?;
        encoded_bytes += bytes;
        if digest != cert.winners_sha256 {
            return Err("content_namespace_winner_digest_mismatch".into());
        }
    }
    for a in certificates {
        for e in &a.excluded_winners {
            for b in certificates {
                if b.scope().matches(&e.normalized_game_relative_path)
                    && !b.excluded_winners.contains(e)
                {
                    return Err("content_namespace_overlapping_exclusion_contradiction".into());
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn require_namespace(
    certificates: &[ContentNamespaceCertificate],
    required: impl Iterator<Item = UsedContentKey>,
    scope: ContentNamespaceScope,
) -> Result<Vec<UsedContentKey>, ContentNamespaceRefusal> {
    scope.validate().map_err(
        |witness| ContentNamespaceRefusal::ContradictoryCertificate {
            scope: scope.clone(),
            witness,
        },
    )?;
    let mut covered = false;
    for certificate in certificates {
        if certificate.scope().covers(&scope) {
            covered = true;
            if let Some(e) = certificate
                .excluded_winners
                .iter()
                .find(|e| scope.matches(&e.normalized_game_relative_path))
            {
                return Err(ContentNamespaceRefusal::ExcludedWinner {
                    scope,
                    path: e.normalized_game_relative_path.clone(),
                    cause: e.cause.clone(),
                });
            }
        }
    }
    if !covered {
        return Err(ContentNamespaceRefusal::Unwitnessed { scope });
    }
    Ok(required
        .filter(|r| scope.matches(&r.normalized_game_relative_path))
        .collect())
}

#[derive(Clone, Debug)]
pub struct ContentNamespaceAuthority {
    pub(crate) certificates: Vec<ContentNamespaceCertificate>,
}

impl ContentNamespaceAuthority {
    pub fn unwitnessed() -> Self {
        Self {
            certificates: Vec::new(),
        }
    }

    pub fn require_scope(
        &self,
        scope: ContentNamespaceScope,
    ) -> Result<(), ContentNamespaceRefusal> {
        require_namespace(&self.certificates, std::iter::empty(), scope).map(|_| ())
    }
}

pub struct ObservedContentNamespace {
    authority: String,
    scope: ContentNamespaceScope,
    rows: Vec<UsedContentKey>,
    excluded: Vec<ExcludedNamespaceWinner>,
    payload_bytes: u64,
    limits: ContentNamespaceLimits,
}

pub struct SealedContentNamespace {
    observation: ObservedContentNamespace,
}

impl ObservedContentNamespace {
    pub fn winners(&self) -> &[UsedContentKey] {
        &self.rows
    }
    pub fn excluded_winners(&self) -> &[ExcludedNamespaceWinner] {
        &self.excluded
    }
    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }

    pub fn seal(self, after_vfs: &EffectiveVfs) -> Result<SealedContentNamespace, String> {
        self.seal_with_progress(after_vfs, &mut |_| {})
    }

    pub fn seal_with_progress(
        self,
        after_vfs: &EffectiveVfs,
        progress: &mut dyn FnMut(usize),
    ) -> Result<SealedContentNamespace, String> {
        let mut budget = ContentNamespaceBudget::new(self.limits)?;
        self.seal_with_budget(after_vfs, &mut budget, progress)
    }

    pub fn seal_with_budget(
        self,
        after_vfs: &EffectiveVfs,
        budget: &mut ContentNamespaceBudget,
        progress: &mut dyn FnMut(usize),
    ) -> Result<SealedContentNamespace, String> {
        let limits = ContentNamespaceLimits {
            winners: self.limits.winners.min(budget.limits.winners),
            metadata_entries: self
                .limits
                .metadata_entries
                .min(budget.limits.metadata_entries),
            roster_bytes: self.limits.roster_bytes.min(budget.limits.roster_bytes),
            payload_bytes: self.limits.payload_bytes.min(budget.limits.payload_bytes),
        };
        let after = after_vfs.observe_namespace_budgeted(&self.scope, budget, limits, progress)?;
        if self.authority != after.authority
            || self.rows != after.rows
            || self.excluded != after.excluded
        {
            return Err("content_namespace_changed_during_census".into());
        }
        Ok(SealedContentNamespace { observation: self })
    }
}

impl SealedContentNamespace {
    fn certificate(&self) -> Result<ContentNamespaceCertificate, String> {
        let observed = &self.observation;
        let rows = observed.rows.iter().collect::<Vec<_>>();
        let (digest, _) = commitment(
            &observed.scope,
            &observed.authority,
            &observed.excluded,
            &rows,
            MAX_NAMESPACE_ROSTER_BYTES,
        )?;
        Ok(ContentNamespaceCertificate {
            directory: observed.scope.directory.clone(),
            extension: observed.scope.extension.clone(),
            recursive: observed.scope.recursive,
            winner_count: observed.rows.len() as u32,
            winners_sha256: digest,
            excluded_winners: observed.excluded.clone(),
        })
    }

    pub fn namespace_authority(&self) -> Result<ContentNamespaceAuthority, String> {
        Ok(ContentNamespaceAuthority {
            certificates: vec![self.certificate()?],
        })
    }

    pub fn winners(&self) -> &[UsedContentKey] {
        self.observation.winners()
    }
    pub fn excluded_winners(&self) -> &[ExcludedNamespaceWinner] {
        self.observation.excluded_winners()
    }
    pub fn payload_bytes(&self) -> u64 {
        self.observation.payload_bytes()
    }
}

impl EffectiveVfs {
    pub fn observe_namespace(
        &self,
        scope: &ContentNamespaceScope,
    ) -> Result<ObservedContentNamespace, String> {
        self.observe_namespace_with_progress(scope, &mut |_| {})
    }

    pub fn observe_namespace_with_progress(
        &self,
        scope: &ContentNamespaceScope,
        progress: &mut dyn FnMut(usize),
    ) -> Result<ObservedContentNamespace, String> {
        let mut budget = ContentNamespaceBudget::new(ContentNamespaceLimits::default())?;
        self.observe_namespace_with_budget(scope, &mut budget, progress)
    }

    pub fn observe_namespace_with_budget(
        &self,
        scope: &ContentNamespaceScope,
        budget: &mut ContentNamespaceBudget,
        progress: &mut dyn FnMut(usize),
    ) -> Result<ObservedContentNamespace, String> {
        let limits = budget.limits;
        self.observe_namespace_budgeted(scope, budget, limits, progress)
    }

    fn observe_namespace_budgeted(
        &self,
        scope: &ContentNamespaceScope,
        budget: &mut ContentNamespaceBudget,
        limits: ContentNamespaceLimits,
        progress: &mut dyn FnMut(usize),
    ) -> Result<ObservedContentNamespace, String> {
        scope.validate()?;
        let enumeration = crate::effective_vfs::NamespaceEnumerationLimits {
            winners: limits.winners.min(budget.limits.winners - budget.winners) + 3,
            entries: limits.metadata_entries,
            bytes: limits.roster_bytes,
        };
        let fresh = self.reopen_bounded(enumeration)?;
        let entries = fresh.scoped_entries_bounded(scope, enumeration)?;
        let mut rows = Vec::new();
        let mut excluded = Vec::new();
        let mut buffer = Vec::new();
        let mut payload_bytes = 0u64;
        for entry in entries {
            let path = crate::normalize_game_relative_path(&entry.logical_path)?;
            if !scope.matches(&path) {
                continue;
            }
            if crate::is_engine_written_install_root_sink(&path, entry.layer_kind) {
                excluded.push(fresh.excluded_namespace_winner(&entry)?);
                continue;
            }
            if rows.len() == limits.winners || budget.winners == budget.limits.winners {
                return Err("content_namespace_bound_exceeded: winners".into());
            }
            budget.winners += 1;
            let limit = crate::MAX_CONTENT_PAYLOAD_BYTES
                .min(limits.payload_bytes - payload_bytes)
                .min(budget.limits.payload_bytes - budget.payload_bytes);
            let (bytes, digest) = fresh.hash_entry_bounded(
                &entry,
                &mut buffer,
                &mut |bytes| {
                    budget.payload_bytes += bytes as u64;
                    progress(bytes);
                },
                limit,
            )?;
            if let EffectiveVfsEntryFacts::File { len, .. } = entry.facts {
                if len != bytes {
                    return Err("content_namespace_file_length_changed".into());
                }
            }
            payload_bytes = payload_bytes
                .checked_add(bytes)
                .filter(|b| *b <= limits.payload_bytes)
                .ok_or("content_namespace_bound_exceeded: payload bytes")?;
            rows.push(fresh.content_key(&entry, digest, bytes)?);
        }
        rows.sort_by(|a, b| {
            a.normalized_game_relative_path
                .cmp(&b.normalized_game_relative_path)
        });
        excluded.sort_by(|a, b| {
            a.normalized_game_relative_path
                .cmp(&b.normalized_game_relative_path)
        });
        let refs: Vec<_> = rows.iter().collect();
        ordered_rows(&refs)?;
        commitment(
            scope,
            fresh.authority_manifest_sha256(),
            &excluded,
            &refs,
            limits.roster_bytes,
        )?;
        Ok(ObservedContentNamespace {
            authority: fresh.authority_manifest_sha256().into(),
            scope: scope.clone(),
            rows,
            excluded,
            payload_bytes,
            limits,
        })
    }
}

impl ContentManifest {
    pub fn bind_observed_namespace(
        &mut self,
        sealed: SealedContentNamespace,
    ) -> Result<(), String> {
        let certificate = sealed.certificate()?;
        let observed = sealed.observation;
        if observed.authority != self.effective_vfs_manifest_sha256 {
            return Err("content_namespace_vfs_identity_mismatch".into());
        }
        let rows: Vec<_> = self
            .required_content
            .iter()
            .map(|r| &r.content)
            .filter(|r| observed.scope.matches(&r.normalized_game_relative_path))
            .collect();
        if rows.iter().copied().ne(observed.rows.iter()) {
            return Err("content_namespace_required_roster_disagrees_with_observation".into());
        }
        let mut certificates = self.witnessed_content_namespaces.clone();
        certificates.push(certificate);
        certificates.sort_by_key(|c| c.scope());
        validate_certificates(
            &certificates,
            &self.required_content,
            &self.effective_vfs_manifest_sha256,
            &self.ordered_layers,
        )?;
        self.witnessed_content_namespaces = certificates;
        self.manifest_digest_sha256 = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effective_vfs::EffectiveVfsLayerMount;
    use crate::{LayerStorageKind, RequiredContentKind, SourceGameIdentity, SourceProfileIdentity};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "namespace_manifest_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn write(&self, path: &str, bytes: &[u8]) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        fn vfs(&self) -> EffectiveVfs {
            EffectiveVfs::new(
                "a".repeat(64),
                [EffectiveVfsLayerMount {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    storage_kind: LayerStorageKind::Directory,
                    local_source: self.0.clone(),
                    source_container_sha256: None,
                    replace_paths: Vec::new(),
                }],
            )
            .unwrap()
        }
        fn manifest(&self) -> ContentManifest {
            let vfs = self.vfs();
            let sealed = vfs
                .observe_namespace(&ContentNamespaceScope::root())
                .unwrap()
                .seal(&vfs)
                .unwrap();
            let rows = sealed
                .winners()
                .iter()
                .map(|key| RequiredContent {
                    content_key: key.normalized_game_relative_path.clone(),
                    kind: RequiredContentKind::Definition,
                    content: key.clone(),
                    observation_count: 1,
                    consumers: vec!["namespace fixture".into()],
                })
                .collect();
            let mut manifest = ContentManifest {
                schema: crate::SCHEMA.into(),
                product_modset_sha256: "b".repeat(64),
                source_game: SourceGameIdentity {
                    version: "1.19.2".into(),
                    build: "a729".into(),
                    executable_sha256: "c".repeat(64),
                    build_profile: "a729-test".into(),
                },
                source_profile: SourceProfileIdentity {
                    profile_id: "profile".into(),
                    playset_id: "playset".into(),
                    selection_sha256: "d".repeat(64),
                },
                effective_vfs_manifest_sha256: "a".repeat(64),
                ordered_layers: vec![RecordedContentLayer {
                    ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    display_name: "base".into(),
                    descriptor_sha256: None,
                    source_container_sha256: None,
                    storage_kind: LayerStorageKind::Directory,
                    replace_paths: Vec::new(),
                    required_for_playback: !sealed.winners().is_empty(),
                }],
                dlc_selection: Vec::new(),
                localisation_language: "l_english".into(),
                required_settings: BTreeMap::new(),
                recorder_local_settings: BTreeMap::new(),
                required_content: rows,
                witnessed_absent_content: Vec::new(),
                witnessed_content_namespaces: Vec::new(),
                native_graphics_query: Default::default(),
                manifest_digest_sha256: None,
            };
            manifest.bind_observed_namespace(sealed).unwrap();
            manifest.source_profile.selection_sha256 =
                manifest.computed_selection_sha256().unwrap();
            manifest.with_computed_digest().unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            portable_test_fs_retry::remove_dir_all(&self.0).unwrap();
        }
    }

    fn validate(manifest: &ContentManifest) -> Result<(), String> {
        validate_certificates(
            &manifest.witnessed_content_namespaces,
            &manifest.required_content,
            &manifest.effective_vfs_manifest_sha256,
            &manifest.ordered_layers,
        )
    }
    fn scope(directory: &str, extension: Option<&str>, recursive: bool) -> ContentNamespaceScope {
        ContentNamespaceScope {
            directory: directory.into(),
            extension: extension.map(str::to_owned),
            recursive,
        }
    }
    fn require(
        manifest: &ContentManifest,
        scope: ContentNamespaceScope,
    ) -> Result<Vec<UsedContentKey>, ContentNamespaceRefusal> {
        require_namespace(
            &manifest.witnessed_content_namespaces,
            manifest.required_content.iter().map(|r| r.content.clone()),
            scope,
        )
    }

    fn small_budget(
        winners: usize,
        metadata_entries: usize,
        payload_bytes: u64,
    ) -> ContentNamespaceBudget {
        ContentNamespaceBudget::new(ContentNamespaceLimits {
            winners,
            metadata_entries,
            roster_bytes: 1 << 20,
            payload_bytes,
        })
        .unwrap()
    }

    #[test]
    fn scoped_observation_never_walks_unrelated_branches_or_reads_other_extensions() {
        let fixture = Fixture::new();
        fixture.write("interface/A.gfx", b"one");
        fixture.write("interface/nested/B.gfx", b"two");
        fixture.write("interface/ignored.gui", &[0; 100]);
        for index in 0..40 {
            fixture.write(&format!("unrelated/deep/{index}.txt"), &[0; 100]);
        }
        let vfs = fixture.vfs();
        let scope = scope("interface", Some("gfx"), true);
        let mut budget = small_budget(4, 8, 12);
        let mut read = 0;
        let observed = vfs
            .observe_namespace_with_budget(&scope, &mut budget, &mut |n| read += n)
            .unwrap();
        assert_eq!(
            observed
                .winners()
                .iter()
                .map(|r| r.stored_game_relative_path.as_str())
                .collect::<Vec<_>>(),
            ["interface/A.gfx", "interface/nested/B.gfx"]
        );
        fixture.write("unrelated/new.txt", b"outside");
        fixture.write("interface/ignored.gui", b"changed excluded extension");
        let sealed = observed
            .seal_with_budget(&vfs, &mut budget, &mut |n| read += n)
            .unwrap();
        assert_eq!(sealed.winners().len(), 2);
        assert_eq!(
            (budget.observed_winners(), budget.payload_bytes(), read),
            (4, 12, 12)
        );
        assert!(vfs
            .observe_namespace_with_budget(
                &ContentNamespaceScope::root(),
                &mut small_budget(100, 8, 10_000),
                &mut |_| {}
            )
            .err()
            .unwrap()
            .contains("enumerated entries"));
    }

    #[test]
    fn scoped_nonrecursive_empty_and_changed_rosters_have_fresh_seal_custody() {
        let fixture = Fixture::new();
        fixture.write("common/abilities/A.txt", b"a");
        for index in 0..20 {
            fixture.write(&format!("common/abilities/nested/{index}.txt"), b"b");
        }
        let vfs = fixture.vfs();
        let immediate = scope("common/abilities", Some("txt"), false);
        let observed = vfs
            .observe_namespace_with_budget(&immediate, &mut small_budget(2, 5, 2), &mut |_| {})
            .unwrap();
        assert_eq!(observed.winners().len(), 1);
        assert!(vfs
            .observe_namespace_with_budget(
                &scope("common/abilities", Some("txt"), true),
                &mut small_budget(30, 5, 100),
                &mut |_| {}
            )
            .err()
            .unwrap()
            .contains("enumerated entries"));
        observed.seal(&vfs).unwrap();
        let missing = scope("common/missing", Some("txt"), true);
        let empty = vfs
            .observe_namespace_with_budget(&missing, &mut small_budget(0, 2, 0), &mut |_| {})
            .unwrap();
        assert!(empty.winners().is_empty());
        empty.seal(&vfs).unwrap();
        for change in 0..3 {
            let before = vfs.observe_namespace(&immediate).unwrap();
            match change {
                0 => fixture.write("common/abilities/B.txt", b"b"),
                1 => std::fs::remove_file(fixture.0.join("common/abilities/B.txt")).unwrap(),
                _ => fixture.write("common/abilities/A.txt", b"changed"),
            }
            assert_eq!(
                before.seal(&vfs).err().unwrap(),
                "content_namespace_changed_during_census"
            );
        }
        let empty = vfs.observe_namespace(&missing).unwrap();
        fixture.write("common/missing/appeared.txt", b"x");
        assert_eq!(
            empty.seal(&vfs).err().unwrap(),
            "content_namespace_changed_during_census"
        );
    }

    #[test]
    fn namespace_budget_is_shared_across_scopes_and_seals_and_refuses_before_payload_read() {
        let fixture = Fixture::new();
        fixture.write("interface/A.gfx", b"abc");
        fixture.write("localisation/english/A.yml", b"de");
        let vfs = fixture.vfs();
        let gfx = scope("interface", Some("gfx"), true);
        let yml = scope("localisation/english", Some("yml"), true);
        let mut budget = small_budget(4, 8, 10);
        for requested in [&gfx, &yml] {
            vfs.observe_namespace_with_budget(requested, &mut budget, &mut |_| {})
                .unwrap()
                .seal_with_budget(&vfs, &mut budget, &mut |_| {})
                .unwrap();
        }
        assert_eq!((budget.observed_winners(), budget.payload_bytes()), (4, 10));
        let mut read = 0;
        assert!(vfs
            .observe_namespace_with_budget(&gfx, &mut small_budget(1, 8, 2), &mut |n| read += n)
            .err()
            .unwrap()
            .contains("payload bytes before read"));
        assert_eq!(read, 0);
        let mut budget = small_budget(4, 8, 5);
        let first = vfs
            .observe_namespace_with_budget(&gfx, &mut budget, &mut |_| {})
            .unwrap();
        assert!(first
            .seal_with_budget(&vfs, &mut budget, &mut |n| read += n)
            .err()
            .unwrap()
            .contains("payload bytes before read"));
        assert_eq!((read, budget.payload_bytes()), (0, 3));
        let mut budget = small_budget(1, 8, 6);
        let first = vfs
            .observe_namespace_with_budget(&gfx, &mut budget, &mut |_| {})
            .unwrap();
        assert!(first
            .seal_with_budget(&vfs, &mut budget, &mut |n| read += n)
            .err()
            .unwrap()
            .contains("winners"));
        assert_eq!(read, 0);
        for invalid in [
            ContentNamespaceLimits {
                winners: MAX_NAMESPACE_WINNERS + 1,
                ..Default::default()
            },
            ContentNamespaceLimits {
                metadata_entries: MAX_NAMESPACE_REFERENCES + 1,
                ..Default::default()
            },
            ContentNamespaceLimits {
                roster_bytes: MAX_NAMESPACE_ROSTER_BYTES + 1,
                ..Default::default()
            },
            ContentNamespaceLimits {
                payload_bytes: MAX_NAMESPACE_PAYLOAD_BYTES + 1,
                ..Default::default()
            },
        ] {
            assert!(ContentNamespaceBudget::new(invalid)
                .err()
                .unwrap()
                .contains("exceeds production ceiling"));
        }
        assert!(vfs
            .observe_namespace_with_budget(
                &gfx,
                &mut ContentNamespaceBudget::new(ContentNamespaceLimits {
                    roster_bytes: 1,
                    ..Default::default()
                })
                .unwrap(),
                &mut |_| {}
            )
            .err()
            .unwrap()
            .contains("enumeration bytes"));
    }

    #[test]
    fn actual_empty_and_nonempty_censuses_bind_full_roster_and_survive_json() {
        let fixture = Fixture::new();
        let empty = fixture.manifest();
        assert_eq!(empty.witnessed_content_namespaces[0].winner_count, 0);
        assert!(require(&empty, scope("common/terrain", Some("txt"), false))
            .unwrap()
            .is_empty());
        fixture.write("Common/Terrain/A.txt", b"a");
        fixture.write("Common/Terrain/b.txt", b"b");
        let full = fixture.manifest();
        let decoded = ContentManifest::decode_json(&full.encode_json().unwrap()).unwrap();
        assert_eq!(decoded, full);
        let keys = require(&decoded, scope("common/terrain", Some("txt"), false)).unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].stored_game_relative_path, "Common/Terrain/A.txt");
        assert_ne!(empty.manifest_digest_sha256, full.manifest_digest_sha256);
        let mut unobserved = full.clone();
        unobserved.witnessed_content_namespaces.clear();
        let unobserved = unobserved.with_computed_digest().unwrap();
        assert_eq!(unobserved.required_content, full.required_content);
        assert!(matches!(
            require(&unobserved, scope("common/terrain", Some("txt"), false)),
            Err(ContentNamespaceRefusal::Unwitnessed { .. })
        ));
        assert!(matches!(
            require(&unobserved, scope("common/empty", None, true)),
            Err(ContentNamespaceRefusal::Unwitnessed { .. })
        ));
        assert_ne!(
            unobserved.manifest_digest_sha256,
            full.manifest_digest_sha256
        );
    }

    #[test]
    fn roster_commitment_rejects_drop_substitution_order_and_every_content_identity_operand() {
        let fixture = Fixture::new();
        fixture.write("Common/Terrain/A.txt", b"a");
        fixture.write("Common/Terrain/b.txt", b"b");
        let original = fixture.manifest();
        let mut dropped = original.clone();
        dropped.required_content.pop();
        assert!(validate(&dropped).unwrap_err().contains("count_mismatch"));
        let mut reordered = original.clone();
        reordered.required_content.reverse();
        assert!(validate(&reordered).unwrap_err().contains("order"));
        for field in 0..9 {
            let mut bad = original.clone();
            let key = &mut bad.required_content[0].content;
            match field {
                0 => key.effective_vfs_manifest_sha256 = "e".repeat(64),
                1 => key.stored_game_relative_path = "common/terrain/a.txt".into(),
                2 => key.normalized_game_relative_path = "common/terrain/0.txt".into(),
                3 => key.winning_layer_ordinal = 1,
                4 => key.layer_kind = ContentLayerKind::Mod,
                5 => key.layer_id = "other".into(),
                6 => key.source_sha256 = "e".repeat(64),
                7 => key.payload_sha256 = "e".repeat(64),
                _ => key.payload_bytes += 1,
            }
            assert!(
                validate(&bad).unwrap_err().contains("digest_mismatch"),
                "field={field}"
            );
        }
        let vfs = fixture.vfs();
        let sealed = vfs
            .observe_namespace(&ContentNamespaceScope::root())
            .unwrap()
            .seal(&vfs)
            .unwrap();
        let mut partial = original;
        partial.required_content.pop();
        partial.witnessed_content_namespaces.clear();
        assert!(partial
            .bind_observed_namespace(sealed)
            .unwrap_err()
            .contains("required_roster_disagrees"));
    }

    #[test]
    fn namespace_grammar_requires_collection_nullable_extension_and_closed_fields() {
        let fixture = Fixture::new();
        let manifest = fixture.manifest();
        for edit in 0..5 {
            let mut json = serde_json::to_value(&manifest).unwrap();
            match edit {
                0 => {
                    json.as_object_mut()
                        .unwrap()
                        .remove("witnessed_content_namespaces");
                }
                1 => json["witnessed_content_namespaces"] = serde_json::Value::Null,
                2 => {
                    json["witnessed_content_namespaces"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("extension");
                }
                3 => json["witnessed_content_namespaces"][0]["unknown"] = true.into(),
                _ => json["schema"] = "hoi4.portable-content-manifest.v4".into(),
            }
            assert!(
                ContentManifest::decode_json(&serde_json::to_vec(&json).unwrap()).is_err(),
                "edit={edit}"
            );
        }
        let explicit_null = serde_json::json!({"directory":"", "extension":null, "recursive":true});
        assert_eq!(
            serde_json::from_value::<ContentNamespaceScope>(explicit_null).unwrap(),
            ContentNamespaceScope::root()
        );
    }

    #[test]
    fn exclusion_intersections_refuse_without_poisoning_unrelated_namespace() {
        let fixture = Fixture::new();
        fixture.write("imgui.ini", b"engine scratch");
        fixture.write("common/terrain/a.txt", b"a");
        let manifest = fixture.manifest();
        assert_eq!(
            manifest.witnessed_content_namespaces[0]
                .excluded_winners
                .len(),
            1
        );
        assert_eq!(
            require(&manifest, scope("common/terrain", Some("txt"), false))
                .unwrap()
                .len(),
            1
        );
        assert!(matches!(
            require(&manifest, ContentNamespaceScope::root()),
            Err(ContentNamespaceRefusal::ExcludedWinner { .. })
        ));
        assert!(require(&manifest, scope("", Some("txt"), true)).is_ok());
        assert!(matches!(
            require(&manifest, scope("", Some("ini"), false)),
            Err(ContentNamespaceRefusal::ExcludedWinner { .. })
        ));
        let mut changed = manifest;
        changed.witnessed_content_namespaces[0].excluded_winners[0].layer_id = "mod".into();
        assert!(validate(&changed).unwrap_err().contains("excluded_winner"));
    }

    #[test]
    fn seal_reopens_and_rehashes_instead_of_trusting_old_file_metadata() {
        for mutation in 0..3 {
            let fixture = Fixture::new();
            fixture.write("common/terrain/a.txt", b"abc");
            let vfs = fixture.vfs();
            let before = vfs
                .observe_namespace(&ContentNamespaceScope::root())
                .unwrap();
            match mutation {
                0 => fixture.write("common/terrain/a.txt", b"xyz"),
                1 => fixture.write("common/terrain/b.txt", b"abc"),
                _ => std::fs::remove_file(fixture.0.join("common/terrain/a.txt")).unwrap(),
            }
            assert!(before
                .seal(&vfs)
                .err()
                .unwrap()
                .contains("changed_during_census"));
        }
    }

    #[test]
    fn coverage_respects_directory_boundaries_depth_and_extension() {
        let immediate = scope("common/terrain", Some("txt"), false);
        assert!(immediate.covers(&immediate));
        for request in [
            scope("common/terrain", None, false),
            scope("common/terrain", Some("txt"), true),
            scope("common/terrain/child", Some("txt"), false),
            scope("common/terrain2", Some("txt"), false),
        ] {
            assert!(!immediate.covers(&request));
        }
        assert!(scope("common", None, true).covers(&immediate));
        assert!(!scope("common", None, true).covers(&scope("common2", None, false)));
    }

    #[test]
    fn actual_400000_winner_and_64_certificate_limits_are_enforced() {
        let fixture = Fixture::new();
        fixture.write("a.txt", b"a");
        let manifest = fixture.manifest();
        let key = &manifest.required_content[0].content;
        let mut keys = (0..MAX_NAMESPACE_WINNERS)
            .map(|i| {
                let mut key = key.clone();
                key.normalized_game_relative_path = format!("{i:06}.txt");
                key
            })
            .collect::<Vec<_>>();
        ordered_rows(&keys.iter().collect::<Vec<_>>()).unwrap();
        keys.push(key.clone());
        assert!(ordered_rows(&keys.iter().collect::<Vec<_>>())
            .unwrap_err()
            .contains("winner count"));
        drop(keys);
        let mut certificates = Vec::new();
        for i in 0..MAX_NAMESPACE_CERTIFICATES {
            let scope = scope(&format!("empty/{i:02}"), None, true);
            let (digest, _) = commitment(
                &scope,
                &manifest.effective_vfs_manifest_sha256,
                &[],
                &[],
                MAX_NAMESPACE_ROSTER_BYTES,
            )
            .unwrap();
            certificates.push(ContentNamespaceCertificate {
                directory: scope.directory,
                extension: scope.extension,
                recursive: scope.recursive,
                winner_count: 0,
                winners_sha256: digest,
                excluded_winners: Vec::new(),
            });
        }
        validate_certificates(
            &certificates,
            &[],
            &manifest.effective_vfs_manifest_sha256,
            &manifest.ordered_layers,
        )
        .unwrap();
        certificates.push(certificates[0].clone());
        assert!(validate_certificates(
            &certificates,
            &[],
            &manifest.effective_vfs_manifest_sha256,
            &manifest.ordered_layers
        )
        .unwrap_err()
        .contains("certificate count"));
    }

    #[test]
    fn aggregate_reference_limit_is_checked_before_materializing_rosters() {
        let fixture = Fixture::new();
        let manifest = fixture.manifest();
        let mut certs = vec![manifest.witnessed_content_namespaces[0].clone(); 3];
        for (i, cert) in certs.iter_mut().enumerate() {
            cert.directory = format!("scope{i}");
        }
        certs[0].winner_count = MAX_NAMESPACE_WINNERS as u32;
        certs[1].winner_count = MAX_NAMESPACE_WINNERS as u32;
        certs[2].winner_count = (MAX_NAMESPACE_REFERENCES - 2 * MAX_NAMESPACE_WINNERS) as u32;
        let at_limit = validate_certificates(
            &certs,
            &[],
            &manifest.effective_vfs_manifest_sha256,
            &manifest.ordered_layers,
        )
        .unwrap_err();
        assert!(at_limit.contains("winner_count_mismatch"), "{at_limit}");
        certs[2].winner_count += 1;
        let over = validate_certificates(
            &certs,
            &[],
            &manifest.effective_vfs_manifest_sha256,
            &manifest.ordered_layers,
        )
        .unwrap_err();
        assert!(over.contains("aggregate references"), "{over}");
    }

    #[test]
    fn actual_aggregate_canonical_byte_limit_is_checked_before_writing_excess() {
        let mut writer = DigestWriter {
            hash: Sha256::new(),
            bytes: 0,
            limit: MAX_NAMESPACE_ROSTER_BYTES,
        };
        let chunk = vec![0u8; 1 << 20];
        for _ in 0..512 {
            writer.write_all(&chunk).unwrap();
        }
        assert_eq!(writer.bytes, MAX_NAMESPACE_ROSTER_BYTES);
        assert!(writer
            .write_all(&[1])
            .unwrap_err()
            .to_string()
            .contains("canonical bytes"));
        assert_eq!(writer.bytes, MAX_NAMESPACE_ROSTER_BYTES);
    }
}
