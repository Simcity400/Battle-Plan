//! Product-package authority for the standalone replay application.
//!
//! This crate deliberately owns no window, tray, downloader, network client,
//! signing key, or installer UI. It verifies an immutable version directory,
//! resolves the executable a stable launcher may start, and atomically switches
//! a small local active-release pointer after verification. A downloader or
//! enterprise installer may stage bytes, but cannot bypass these gates.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const PACKAGE_SCHEMA: &str = "hoi4.replay.package.v1";
pub const PRODUCT_ID: &str = "battle-plan";
pub const PACKAGE_MANIFEST_FILE: &str = "replay-package.json";
pub const ACTIVE_RELEASE_FILE: &str = "active-release.json";
pub const VERSIONS_DIR: &str = "versions";
pub const STABLE_LAUNCHER_FILE: &str = "replay_launcher.exe";

const ACTIVE_SCHEMA_VERSION: u32 = 1;
const UPDATE_LOCK_FILE: &str = ".package-update.lock";
const MAX_MANIFEST_BYTES: u64 = 1 << 20;
const MAX_PACKAGE_FILES: usize = 4096;
const MAX_PACKAGE_DIRECTORIES: usize = 4096;
const MAX_PACKAGE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: usize = 16 * 1024;
static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub executable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSignature {
    pub algorithm: String,
    pub signer_id: String,
    pub payload_sha256: String,
    /// Detached signature bytes encoded as lowercase hexadecimal.
    pub signature_hex: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    pub schema: String,
    pub product_id: String,
    pub release_version: String,
    pub target: String,
    pub entrypoint: String,
    pub files: Vec<PackageFile>,
    pub package_digest_sha256: String,
    pub signature: Option<PackageSignature>,
}

#[derive(Serialize)]
struct UnsignedManifest<'a> {
    schema: &'a str,
    product_id: &'a str,
    release_version: &'a str,
    target: &'a str,
    entrypoint: &'a str,
    files: &'a [PackageFile],
}

impl PackageManifest {
    pub fn parse_canonical(bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err("package manifest exceeds the 1 MiB cap".into());
        }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("parse package manifest: {error}"))?;
        manifest.validate_shape()?;
        let canonical = manifest.canonical_bytes()?;
        if canonical != bytes {
            return Err(
                "package manifest is not the exact canonical JSON encoding; duplicate, reordered, or padded input is refused"
                    .into(),
            );
        }
        Ok(manifest)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate_shape()?;
        serde_json::to_vec(self).map_err(|error| format!("encode package manifest: {error}"))
    }

    pub fn computed_package_digest(&self) -> Result<String> {
        let unsigned = UnsignedManifest {
            schema: &self.schema,
            product_id: &self.product_id,
            release_version: &self.release_version,
            target: &self.target,
            entrypoint: &self.entrypoint,
            files: &self.files,
        };
        let bytes = serde_json::to_vec(&unsigned)
            .map_err(|error| format!("encode unsigned package manifest: {error}"))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn validate_shape(&self) -> Result<()> {
        if self.schema != PACKAGE_SCHEMA {
            return Err(format!("unsupported package schema {:?}", self.schema));
        }
        if self.product_id != PRODUCT_ID {
            return Err(format!(
                "unexpected package product id {:?}",
                self.product_id
            ));
        }
        validate_token(&self.release_version, "release_version")?;
        validate_token(&self.target, "target")?;
        validate_package_path(&self.entrypoint)?;
        validate_sha256(&self.package_digest_sha256, "package digest")?;
        if self.files.is_empty() || self.files.len() > MAX_PACKAGE_FILES {
            return Err(format!(
                "package file count must be in 1..={MAX_PACKAGE_FILES}"
            ));
        }

        let mut previous = None;
        let mut total = 0u64;
        let mut entrypoint_rows = 0usize;
        let mut collision_keys = BTreeSet::new();
        for file in &self.files {
            validate_package_path(&file.path)?;
            validate_sha256(&file.sha256, "package file digest")?;
            if file.path.eq_ignore_ascii_case(PACKAGE_MANIFEST_FILE)
                || file.bytes > MAX_PACKAGE_BYTES
            {
                return Err(format!("invalid package file row for {:?}", file.path));
            }
            if previous.is_some_and(|path: &str| path >= file.path.as_str()) {
                return Err("package file rows must be unique and sorted by path".into());
            }
            previous = Some(file.path.as_str());
            if !collision_keys.insert(file.path.to_ascii_lowercase()) {
                return Err(format!("package path aliases another row: {:?}", file.path));
            }
            total = total
                .checked_add(file.bytes)
                .ok_or("package byte total overflow")?;
            if total > MAX_PACKAGE_BYTES {
                return Err(format!("package exceeds {MAX_PACKAGE_BYTES} bytes"));
            }
            if file.path == self.entrypoint {
                entrypoint_rows += 1;
                if !file.executable {
                    return Err("package entrypoint is not marked executable".into());
                }
            }
        }
        if entrypoint_rows != 1 {
            return Err("package entrypoint must identify exactly one file row".into());
        }
        let computed = self.computed_package_digest()?;
        if computed != self.package_digest_sha256 {
            return Err(format!(
                "package digest mismatch: computed {computed}, declared {}",
                self.package_digest_sha256
            ));
        }
        if let Some(signature) = &self.signature {
            validate_token(&signature.algorithm, "signature algorithm")?;
            validate_token(&signature.signer_id, "signature signer_id")?;
            validate_sha256(&signature.payload_sha256, "signature payload digest")?;
            if signature.payload_sha256 != self.package_digest_sha256 {
                return Err("signature payload digest does not bind this package".into());
            }
            if signature.signature_hex.len() > MAX_SIGNATURE_BYTES * 2 {
                return Err(format!(
                    "detached package signature exceeds {MAX_SIGNATURE_BYTES} bytes"
                ));
            }
            decode_lower_hex(&signature.signature_hex, "detached package signature")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureRequirement {
    Required,
    AllowUnsignedDevelopment,
}

/// Explicit trust policy supplied by the product launcher or installer.
///
/// There is intentionally no `Default`: production must choose its accepted
/// algorithms and signer ids, while unsigned development mode must be requested
/// by name at the call site.
#[derive(Clone, Debug)]
pub struct SigningPolicy {
    requirement: SignatureRequirement,
    trusted_signers: BTreeMap<String, BTreeSet<String>>,
}

impl SigningPolicy {
    pub fn production(trusted_signers: BTreeMap<String, BTreeSet<String>>) -> Result<Self> {
        if trusted_signers.is_empty()
            || trusted_signers
                .iter()
                .any(|(algorithm, signers)| algorithm.is_empty() || signers.is_empty())
        {
            return Err(
                "production signing policy requires an algorithm and signer allowlist".into(),
            );
        }
        for (algorithm, signers) in &trusted_signers {
            validate_token(algorithm, "trusted signature algorithm")?;
            for signer in signers {
                validate_token(signer, "trusted signer id")?;
            }
        }
        Ok(Self {
            requirement: SignatureRequirement::Required,
            trusted_signers,
        })
    }

    pub fn unsigned_development() -> Self {
        Self {
            requirement: SignatureRequirement::AllowUnsignedDevelopment,
            trusted_signers: BTreeMap::new(),
        }
    }
}

pub trait PackageSignatureVerifier {
    /// Verify a detached signature over exactly the 32 raw bytes represented
    /// by `PackageManifest::package_digest_sha256` (not its hexadecimal text).
    fn verify(
        &self,
        algorithm: &str,
        signer_id: &str,
        payload_sha256: &[u8; 32],
        signature: &[u8],
    ) -> Result<()>;
}

/// Product-owned version/channel/rollback policy. Cryptographic validity alone
/// does not decide whether a signed release is currently admissible or is an
/// allowed transition. There is deliberately no permissive implementation in
/// this crate: the stable launcher and updater must supply the product rules.
pub trait ReleaseAdmissionPolicy {
    fn admit_active(&self, active: &PackageManifest) -> Result<()>;

    fn admit_promotion(
        &self,
        current: Option<&PackageManifest>,
        candidate: &PackageManifest,
    ) -> Result<()>;

    fn admit_rollback(&self, current: &PackageManifest, previous: &PackageManifest) -> Result<()>;
}

pub fn admit_signature(
    manifest: &PackageManifest,
    policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
) -> Result<()> {
    manifest.validate_shape()?;
    let Some(signature) = &manifest.signature else {
        return match policy.requirement {
            SignatureRequirement::Required => {
                Err("production package has no trusted detached signature".into())
            }
            SignatureRequirement::AllowUnsignedDevelopment => Ok(()),
        };
    };
    let allowed = policy
        .trusted_signers
        .get(&signature.algorithm)
        .is_some_and(|signers| signers.contains(&signature.signer_id));
    if !allowed {
        return Err(format!(
            "package signer {:?} using {:?} is not trusted by this policy",
            signature.signer_id, signature.algorithm
        ));
    }
    let verifier = verifier
        .ok_or("signed package cannot be admitted because no signature verifier is configured")?;
    let payload = decode_sha256(&signature.payload_sha256, "signature payload digest")?;
    let bytes = decode_lower_hex(&signature.signature_hex, "detached package signature")?;
    verifier.verify(&signature.algorithm, &signature.signer_id, &payload, &bytes)
}

#[derive(Clone, Debug)]
pub struct RuntimeIdentity {
    pub product_id: String,
    pub target: String,
}

impl RuntimeIdentity {
    pub fn current() -> Self {
        Self {
            product_id: PRODUCT_ID.into(),
            target: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        }
    }
}

#[derive(Clone, Debug)]
pub struct VerifiedRelease {
    pub root: PathBuf,
    pub manifest: PackageManifest,
    pub entrypoint: PathBuf,
}

/// Result of durably retaining a verified staging tree in the immutable,
/// content-addressed version store. Retention deliberately does not update the
/// active-release pointer; activation remains a separate policy-gated
/// transaction through [`promote_release`].
#[derive(Clone, Debug)]
pub struct RetainedRelease {
    pub release: VerifiedRelease,
    pub already_present: bool,
}

/// Product-owned process-creation boundary used by the stable launcher.
///
/// This crate deliberately supplies no default implementation. In production,
/// the implementation must create `entrypoint` directly (never through a
/// shell), use `working_directory`, apply the chosen platform DLL-search and
/// process-mitigation policy, and preserve `forwarded_args` as arguments rather
/// than reparsing them as a command line. The install ACL must prevent package
/// mutation between verification and this call.
pub trait AdmittedEntrypointLauncher {
    type Output;

    fn launch(
        &self,
        entrypoint: &Path,
        working_directory: &Path,
        forwarded_args: &[OsString],
    ) -> Result<Self::Output>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LauncherFailure {
    /// The stable-launcher identity, installed layout, active pointer, package,
    /// signature, runtime identity, or release-admission gate refused launch.
    Preflight(String),
    /// All preflight gates passed and the product-owned direct process creation
    /// callback was attempted, but that callback failed.
    ProcessCreation(String),
}

impl std::fmt::Display for LauncherFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preflight(message) => write!(formatter, "launcher preflight refused: {message}"),
            Self::ProcessCreation(message) => {
                write!(formatter, "verified entrypoint process creation failed: {message}")
            }
        }
    }
}

impl std::error::Error for LauncherFailure {}

pub type LauncherResult<T> = std::result::Result<T, LauncherFailure>;

/// Resolve the deterministic install root containing the stable launcher.
///
/// The launcher itself is outside every replaceable version directory. Its
/// platform signature and the ACL on this root are product/deployment policy;
/// this function only refuses path aliases and the wrong executable name.
pub fn install_root_from_launcher(launcher_executable: &Path) -> Result<PathBuf> {
    refuse_symlink(launcher_executable, "stable launcher")?;
    let launcher = fs::canonicalize(launcher_executable).map_err(|error| {
        format!(
            "canonicalize stable launcher {}: {error}",
            launcher_executable.display()
        )
    })?;
    if !launcher.is_file() {
        return Err(format!(
            "stable launcher is not a regular file: {}",
            launcher.display()
        ));
    }
    let launcher_file = File::open(&launcher)
        .map_err(|error| format!("open stable launcher {}: {error}", launcher.display()))?;
    refuse_multi_link_file(&launcher_file, &launcher)?;
    let file_name = launcher
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("stable launcher has no portable UTF-8 file name")?;
    if !file_name.eq_ignore_ascii_case(STABLE_LAUNCHER_FILE) {
        return Err(format!(
            "stable launcher must be named {STABLE_LAUNCHER_FILE:?}, found {file_name:?}"
        ));
    }
    let install_root = launcher
        .parent()
        .ok_or("stable launcher has no install root")?;
    refuse_symlink(install_root, "stable launcher install root")?;
    fs::canonicalize(install_root).map_err(|error| {
        format!(
            "canonicalize stable launcher install root {}: {error}",
            install_root.display()
        )
    })
}

/// Verify the active package through every trust/admission gate and hand only
/// its admitted entrypoint to the product-owned process launcher.
///
/// `forwarded_args` are opaque OS arguments. This layer never interprets a
/// recording path, guesses HOI4/user-profile locations, invokes a shell, or
/// accepts a caller-selected executable. A failure before the final callback
/// guarantees that the callback is not invoked.
pub fn invoke_active_release_from_current_launcher<L: AdmittedEntrypointLauncher>(
    forwarded_args: &[OsString],
    identity: &RuntimeIdentity,
    signing_policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
    release_policy: &dyn ReleaseAdmissionPolicy,
    process_launcher: &L,
) -> LauncherResult<L::Output> {
    let launcher_executable = std::env::current_exe()
        .map_err(|error| {
            LauncherFailure::Preflight(format!(
                "resolve current stable launcher executable: {error}"
            ))
        })?;
    invoke_active_release_from_launcher_path(
        &launcher_executable,
        forwarded_args,
        identity,
        signing_policy,
        verifier,
        release_policy,
        process_launcher,
    )
}

/// Stable-launcher CLI adapter: forward every native argument after argv[0] to
/// the admitted replay entrypoint without UTF-8 conversion or shell parsing.
///
/// This crate defines no launcher-owned update/channel/debug switches. Adding
/// such switches is product policy and must not make a failed package gate
/// recover to an alternate executable.
pub fn invoke_active_release_from_current_cli<L: AdmittedEntrypointLauncher>(
    identity: &RuntimeIdentity,
    signing_policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
    release_policy: &dyn ReleaseAdmissionPolicy,
    process_launcher: &L,
) -> LauncherResult<L::Output> {
    let forwarded_args = std::env::args_os().skip(1).collect::<Vec<_>>();
    invoke_active_release_from_current_launcher(
        &forwarded_args,
        identity,
        signing_policy,
        verifier,
        release_policy,
        process_launcher,
    )
}

fn invoke_active_release_from_launcher_path<L: AdmittedEntrypointLauncher>(
    launcher_executable: &Path,
    forwarded_args: &[OsString],
    identity: &RuntimeIdentity,
    signing_policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
    release_policy: &dyn ReleaseAdmissionPolicy,
    process_launcher: &L,
) -> LauncherResult<L::Output> {
    let install_root = install_root_from_launcher(launcher_executable)
        .map_err(LauncherFailure::Preflight)?;
    let release = discover_active_release(
        &install_root,
        identity,
        signing_policy,
        verifier,
        release_policy,
    )
    .map_err(LauncherFailure::Preflight)?;
    process_launcher
        .launch(&release.entrypoint, &release.root, forwarded_args)
        .map_err(LauncherFailure::ProcessCreation)
}

/// Construct the canonical unsigned manifest payload for a completely staged
/// directory. Signing remains an external release-pipeline operation; this
/// function only centralizes deterministic enumeration and hashing.
///
/// The directory must not already contain `replay-package.json`, which avoids
/// accidentally treating a stale manifest as package payload on a rebuild.
pub fn build_unsigned_manifest(
    root: &Path,
    release_version: &str,
    target: &str,
    entrypoint: &str,
) -> Result<PackageManifest> {
    validate_token(release_version, "release_version")?;
    validate_token(target, "target")?;
    validate_package_path(entrypoint)?;
    refuse_symlink(root, "package staging root")?;
    let root = fs::canonicalize(root).map_err(|error| {
        format!(
            "canonicalize package staging root {}: {error}",
            root.display()
        )
    })?;
    if !root.is_dir() {
        return Err(format!(
            "package staging root is not a directory: {}",
            root.display()
        ));
    }

    let paths = enumerate_package_files(&root)?;
    if paths
        .iter()
        .any(|path| path.eq_ignore_ascii_case(PACKAGE_MANIFEST_FILE))
    {
        return Err("package staging root already contains replay-package.json".into());
    }
    if paths.len() > MAX_PACKAGE_FILES {
        return Err(format!(
            "package staging file count exceeds {MAX_PACKAGE_FILES}"
        ));
    }
    if !paths.contains(entrypoint) {
        return Err(format!(
            "package entrypoint is absent from staging: {entrypoint:?}"
        ));
    }
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let (bytes, sha256) = hash_file(&checked_join(&root, &path)?)?;
        files.push(PackageFile {
            executable: path == entrypoint,
            path,
            bytes,
            sha256,
        });
    }
    let mut manifest = PackageManifest {
        schema: PACKAGE_SCHEMA.into(),
        product_id: PRODUCT_ID.into(),
        release_version: release_version.into(),
        target: target.into(),
        entrypoint: entrypoint.into(),
        files,
        package_digest_sha256: String::new(),
        signature: None,
    };
    manifest.package_digest_sha256 = manifest.computed_package_digest()?;
    manifest.validate_shape()?;
    Ok(manifest)
}

pub fn verify_release_dir(
    root: &Path,
    identity: &RuntimeIdentity,
    policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
) -> Result<VerifiedRelease> {
    refuse_symlink(root, "package root")?;
    let root = fs::canonicalize(root)
        .map_err(|error| format!("canonicalize package root {}: {error}", root.display()))?;
    if !root.is_dir() {
        return Err(format!(
            "package root is not a directory: {}",
            root.display()
        ));
    }
    let manifest_path = root.join(PACKAGE_MANIFEST_FILE);
    refuse_symlink(&manifest_path, "package manifest")?;
    let manifest_bytes = read_capped(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest = PackageManifest::parse_canonical(&manifest_bytes)?;
    if manifest.product_id != identity.product_id || manifest.target != identity.target {
        return Err(format!(
            "package identity/target mismatch: found {}/{}, expected {}/{}",
            manifest.product_id, manifest.target, identity.product_id, identity.target
        ));
    }
    admit_signature(&manifest, policy, verifier)?;

    let actual_paths = enumerate_package_files(&root)?;
    let mut expected_paths = manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    expected_paths.insert(PACKAGE_MANIFEST_FILE.into());
    if actual_paths != expected_paths {
        let missing = expected_paths
            .difference(&actual_paths)
            .cloned()
            .collect::<Vec<_>>();
        let unlisted = actual_paths
            .difference(&expected_paths)
            .cloned()
            .collect::<Vec<_>>();
        return Err(format!(
            "package file coverage mismatch; missing={missing:?}, unlisted={unlisted:?}"
        ));
    }

    for file in &manifest.files {
        let path = checked_join(&root, &file.path)?;
        let (bytes, sha256) = hash_file(&path)?;
        if bytes != file.bytes || sha256 != file.sha256 {
            return Err(format!(
                "package file {:?} disagrees with manifest: bytes {bytes}/{}, sha256 {sha256}/{}",
                file.path, file.bytes, file.sha256
            ));
        }
    }
    let entrypoint = checked_join(&root, &manifest.entrypoint)?;
    Ok(VerifiedRelease {
        root,
        manifest,
        entrypoint,
    })
}

/// Verify the package containing an already-running executable. This supports a
/// direct-development launch without teaching the product any Steam/default
/// installation path.
pub fn verify_sibling_package(
    executable: &Path,
    identity: &RuntimeIdentity,
    policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
) -> Result<VerifiedRelease> {
    refuse_symlink(executable, "running executable")?;
    let executable = fs::canonicalize(executable)
        .map_err(|error| format!("canonicalize executable {}: {error}", executable.display()))?;
    let root = executable
        .parent()
        .ok_or("running executable has no package directory")?;
    let release = verify_release_dir(root, identity, policy, verifier)?;
    let admitted_entrypoint = fs::canonicalize(&release.entrypoint).map_err(|error| {
        format!(
            "canonicalize admitted package entrypoint {}: {error}",
            release.entrypoint.display()
        )
    })?;
    if admitted_entrypoint != executable {
        return Err("running executable is not the admitted package entrypoint".into());
    }
    Ok(release)
}

/// Copy a completely staged, signed release into
/// `versions/<package-digest-sha256>` without making it active.
///
/// The source is verified before copying. Every payload is copied through an
/// exclusively-created file while its byte count and digest are recomputed,
/// then flushed. The canonical signed manifest is written last. The complete
/// incoming tree is verified again and atomically renamed to its digest path.
/// A crash can therefore leave only an inert `.incoming-*` directory; neither
/// discovery nor promotion will select it. Stale-incoming cleanup is an
/// installer retention decision and is intentionally not guessed here.
///
/// If the digest directory already exists it is never rewritten: the existing
/// tree must independently verify or the operation fails closed.
pub fn retain_staged_release(
    install_root: &Path,
    staged_release: &Path,
    identity: &RuntimeIdentity,
    policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
) -> Result<RetainedRelease> {
    let staged = verify_release_dir(staged_release, identity, policy, verifier)?;
    let digest = staged.manifest.package_digest_sha256.clone();
    let _lock = UpdateLock::acquire(install_root)?;

    let versions = install_root.join(VERSIONS_DIR);
    fs::create_dir_all(&versions)
        .map_err(|error| format!("create version store {}: {error}", versions.display()))?;
    refuse_symlink(&versions, "version store")?;
    if !versions.is_dir() {
        return Err(format!(
            "version store is not a directory: {}",
            versions.display()
        ));
    }

    let final_root = versions.join(&digest);
    match fs::symlink_metadata(&final_root) {
        Ok(_) => {
            let release = verify_versioned_release(
                install_root,
                &digest,
                identity,
                policy,
                verifier,
            )?;
            return Ok(RetainedRelease {
                release,
                already_present: true,
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "inspect retained release path {}: {error}",
                final_root.display()
            ))
        }
    }

    let incoming_root = create_unique_incoming_directory(&versions, &digest)?;
    let mut incoming = IncomingDirectory::new(incoming_root);
    copy_verified_release(&staged, incoming.path())?;
    let copied = verify_release_dir(incoming.path(), identity, policy, verifier)?;
    if copied.manifest.package_digest_sha256 != digest {
        return Err("copied release digest changed after staging verification".into());
    }
    atomic_install_directory(incoming.path(), &final_root)?;
    incoming.commit();

    let release = verify_versioned_release(install_root, &digest, identity, policy, verifier)?;
    Ok(RetainedRelease {
        release,
        already_present: false,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveRelease {
    pub schema_version: u32,
    pub product_id: String,
    pub current_digest_sha256: String,
    pub previous_digest_sha256: Option<String>,
    pub generation: u64,
}

impl ActiveRelease {
    fn validate(&self) -> Result<()> {
        if self.schema_version != ACTIVE_SCHEMA_VERSION || self.product_id != PRODUCT_ID {
            return Err("active-release pointer has unsupported schema/product identity".into());
        }
        if self.generation == 0 {
            return Err("active-release generation must be nonzero".into());
        }
        validate_sha256(&self.current_digest_sha256, "active release digest")?;
        if let Some(previous) = &self.previous_digest_sha256 {
            validate_sha256(previous, "previous release digest")?;
            if previous == &self.current_digest_sha256 {
                return Err("active and previous release digests are identical".into());
            }
        }
        Ok(())
    }

    fn parse_canonical(bytes: &[u8]) -> Result<Self> {
        let pointer: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("parse active release pointer: {error}"))?;
        pointer.validate()?;
        if serde_json::to_vec(&pointer).map_err(|error| error.to_string())? != bytes {
            return Err("active release pointer is not canonical JSON".into());
        }
        Ok(pointer)
    }
}

pub fn discover_active_release(
    install_root: &Path,
    identity: &RuntimeIdentity,
    policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
    release_policy: &dyn ReleaseAdmissionPolicy,
) -> Result<VerifiedRelease> {
    let pointer =
        load_active_pointer(install_root)?.ok_or("installation has no active release pointer")?;
    let release = verify_versioned_release(
        install_root,
        &pointer.current_digest_sha256,
        identity,
        policy,
        verifier,
    )?;
    release_policy.admit_active(&release.manifest)?;
    // Verification hashes every package file and may take materially longer
    // than reading the pointer. Refuse a stale launch if an updater promoted or
    // rolled back while this verification was in progress. The next launcher
    // attempt will verify the newly selected digest from the beginning.
    let pointer_after = load_active_pointer(install_root)?
        .ok_or("active release pointer disappeared during verification")?;
    if pointer_after != pointer {
        return Err("active release pointer changed during verification; launch refused".into());
    }
    Ok(release)
}

pub fn promote_release(
    install_root: &Path,
    candidate_digest_sha256: &str,
    identity: &RuntimeIdentity,
    policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
    release_policy: &dyn ReleaseAdmissionPolicy,
) -> Result<ActiveRelease> {
    validate_sha256(candidate_digest_sha256, "candidate release digest")?;
    let _lock = UpdateLock::acquire(install_root)?;
    let candidate = verify_versioned_release(
        install_root,
        candidate_digest_sha256,
        identity,
        policy,
        verifier,
    )?;
    if candidate.manifest.package_digest_sha256 != candidate_digest_sha256 {
        return Err("candidate directory name does not equal its package digest".into());
    }
    // Never write a pointer that the launch path would immediately refuse.
    release_policy.admit_active(&candidate.manifest)?;

    let old = load_active_pointer(install_root)?;
    let mut current_release = None;
    if let Some(current) = &old {
        if current.current_digest_sha256 == candidate_digest_sha256 {
            return Ok(current.clone());
        }
        // Never replace a corrupt active installation and silently discard its
        // forensic/rollback state. Recovery is an explicit operator action.
        current_release = Some(verify_versioned_release(
            install_root,
            &current.current_digest_sha256,
            identity,
            policy,
            verifier,
        )?);
    }
    release_policy.admit_promotion(
        current_release.as_ref().map(|release| &release.manifest),
        &candidate.manifest,
    )?;
    let pointer = ActiveRelease {
        schema_version: ACTIVE_SCHEMA_VERSION,
        product_id: PRODUCT_ID.into(),
        current_digest_sha256: candidate_digest_sha256.into(),
        previous_digest_sha256: old
            .as_ref()
            .map(|value| value.current_digest_sha256.clone()),
        generation: match old.as_ref() {
            Some(value) => value
                .generation
                .checked_add(1)
                .ok_or("release generation overflow")?,
            None => 1,
        },
    };
    write_active_pointer(install_root, &pointer)?;
    Ok(pointer)
}

pub fn rollback_release(
    install_root: &Path,
    identity: &RuntimeIdentity,
    policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
    release_policy: &dyn ReleaseAdmissionPolicy,
) -> Result<ActiveRelease> {
    let _lock = UpdateLock::acquire(install_root)?;
    let old = load_active_pointer(install_root)?
        .ok_or("installation has no active release to roll back")?;
    let previous = old
        .previous_digest_sha256
        .clone()
        .ok_or("active release has no retained rollback target")?;
    let current_release = verify_versioned_release(
        install_root,
        &old.current_digest_sha256,
        identity,
        policy,
        verifier,
    )?;
    let previous_release =
        verify_versioned_release(install_root, &previous, identity, policy, verifier)?;
    // Rollback may have distinct emergency-transition rules, but its result
    // must still be a release the normal launch path admits.
    release_policy.admit_active(&previous_release.manifest)?;
    release_policy.admit_rollback(&current_release.manifest, &previous_release.manifest)?;
    let pointer = ActiveRelease {
        schema_version: ACTIVE_SCHEMA_VERSION,
        product_id: PRODUCT_ID.into(),
        current_digest_sha256: previous,
        previous_digest_sha256: Some(old.current_digest_sha256),
        generation: old
            .generation
            .checked_add(1)
            .ok_or("release generation overflow")?,
    };
    write_active_pointer(install_root, &pointer)?;
    Ok(pointer)
}

fn verify_versioned_release(
    install_root: &Path,
    digest: &str,
    identity: &RuntimeIdentity,
    policy: &SigningPolicy,
    verifier: Option<&dyn PackageSignatureVerifier>,
) -> Result<VerifiedRelease> {
    validate_sha256(digest, "version-directory digest")?;
    refuse_symlink(install_root, "install root")?;
    let root = install_root.join(VERSIONS_DIR).join(digest);
    let release = verify_release_dir(&root, identity, policy, verifier)?;
    if release.manifest.package_digest_sha256 != digest {
        return Err("version directory digest does not bind the admitted package".into());
    }
    Ok(release)
}

fn load_active_pointer(install_root: &Path) -> Result<Option<ActiveRelease>> {
    refuse_symlink(install_root, "install root")?;
    let path = install_root.join(ACTIVE_RELEASE_FILE);
    match fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "inspect active release pointer {}: {error}",
                path.display()
            ))
        }
    }
    refuse_symlink(&path, "active release pointer")?;
    let bytes = read_capped(&path, MAX_MANIFEST_BYTES)?;
    ActiveRelease::parse_canonical(&bytes).map(Some)
}

fn write_active_pointer(install_root: &Path, pointer: &ActiveRelease) -> Result<()> {
    pointer.validate()?;
    fs::create_dir_all(install_root)
        .map_err(|error| format!("create install root {}: {error}", install_root.display()))?;
    refuse_symlink(install_root, "install root")?;
    let bytes = serde_json::to_vec(pointer)
        .map_err(|error| format!("encode active release pointer: {error}"))?;
    write_atomic(&install_root.join(ACTIVE_RELEASE_FILE), &bytes)
}

struct UpdateLock {
    path: PathBuf,
}

impl UpdateLock {
    fn acquire(install_root: &Path) -> Result<Self> {
        fs::create_dir_all(install_root)
            .map_err(|error| format!("create install root {}: {error}", install_root.display()))?;
        refuse_symlink(install_root, "install root")?;
        let path = install_root.join(UPDATE_LOCK_FILE);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                format!(
                    "acquire package update lock {}: {error}; a stale lock requires explicit operator recovery",
                    path.display()
                )
            })?;
        writeln!(file, "pid={}", std::process::id())
            .map_err(|error| format!("write package update lock: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("flush package update lock: {error}"))?;
        Ok(Self { path })
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

struct IncomingDirectory {
    path: PathBuf,
    committed: bool,
}

impl IncomingDirectory {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for IncomingDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn create_unique_incoming_directory(versions: &Path, digest: &str) -> Result<PathBuf> {
    let time_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..64 {
        let counter = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = versions.join(format!(
            ".incoming-{digest}.{}.{}.{}",
            std::process::id(),
            time_nonce,
            counter
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "create incoming release directory {}: {error}",
                    path.display()
                ))
            }
        }
    }
    Err("could not allocate a unique incoming release directory".into())
}

fn copy_verified_release(source: &VerifiedRelease, destination: &Path) -> Result<()> {
    refuse_symlink(destination, "incoming release directory")?;
    for row in &source.manifest.files {
        let source_path = checked_join(&source.root, &row.path)?;
        let destination_path = destination.join(&row.path);
        let parent = destination_path
            .parent()
            .ok_or("incoming package path has no parent")?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create incoming package directory {}: {error}",
                parent.display()
            )
        })?;
        copy_file_exact(&source_path, &destination_path, row.bytes, &row.sha256)?;
    }

    let manifest_bytes = source.manifest.canonical_bytes()?;
    write_new_file_synced(&destination.join(PACKAGE_MANIFEST_FILE), &manifest_bytes)?;
    sync_directory_tree(destination)
}

fn copy_file_exact(source: &Path, destination: &Path, bytes: u64, sha256: &str) -> Result<()> {
    let mut input = File::open(source)
        .map_err(|error| format!("open staged package file {}: {error}", source.display()))?;
    refuse_multi_link_file(&input, source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| {
            format!(
                "create incoming package file {}: {error}",
                destination.display()
            )
        })?;
    let mut hasher = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("read staged package file {}: {error}", source.display()))?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or("incoming package byte count overflow")?;
        if copied > bytes || copied > MAX_PACKAGE_BYTES {
            return Err(format!(
                "staged package file changed size while copying: {}",
                source.display()
            ));
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read]).map_err(|error| {
            format!(
                "write incoming package file {}: {error}",
                destination.display()
            )
        })?;
    }
    let copied_sha256 = format!("{:x}", hasher.finalize());
    if copied != bytes || copied_sha256 != sha256 {
        return Err(format!(
            "staged package file changed after verification: {}",
            source.display()
        ));
    }
    output
        .sync_all()
        .map_err(|error| format!("flush incoming package file {}: {error}", destination.display()))
}

fn write_new_file_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create incoming manifest {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("write incoming manifest {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("flush incoming manifest {}: {error}", path.display()))
}

#[cfg(windows)]
fn sync_directory_tree(_root: &Path) -> Result<()> {
    // Each payload and the manifest have already been flushed. The final
    // directory rename below uses MOVEFILE_WRITE_THROUGH; opening directories
    // for FlushFileBuffers is not supported by std on Windows.
    Ok(())
}

#[cfg(not(windows))]
fn sync_directory_tree(root: &Path) -> Result<()> {
    let mut directories = vec![root.to_path_buf()];
    let mut cursor = 0usize;
    while cursor < directories.len() {
        for entry in fs::read_dir(&directories[cursor]).map_err(|error| {
            format!(
                "enumerate incoming directory {} for flush: {error}",
                directories[cursor].display()
            )
        })? {
            let entry = entry.map_err(|error| format!("enumerate incoming directory: {error}"))?;
            if entry
                .file_type()
                .map_err(|error| format!("inspect incoming directory entry: {error}"))?
                .is_dir()
            {
                directories.push(entry.path());
            }
        }
        cursor += 1;
    }
    for directory in directories.into_iter().rev() {
        File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("flush incoming directory {}: {error}", directory.display()))?;
    }
    Ok(())
}

#[cfg(windows)]
fn atomic_install_directory(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let source = source
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let ok = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok == 0 {
        Err(format!(
            "atomic incoming-directory install failed: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_install_directory(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target).map_err(|error| {
        format!(
            "atomic incoming-directory install {} -> {} failed: {error}",
            source.display(),
            target.display()
        )
    })?;
    let parent = target
        .parent()
        .ok_or("installed version directory has no parent")?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("flush version-store directory {}: {error}", parent.display()))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or("atomic target has no parent directory")?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("atomic target has no UTF-8 file name")?;
    let (temp, mut file) = create_unique_temp(parent, file_name)?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("write atomic temp {}: {error}", temp.display()))?;
        file.sync_all()
            .map_err(|error| format!("flush atomic temp {}: {error}", temp.display()))?;
        drop(file);
        atomic_replace(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn create_unique_temp(parent: &Path, file_name: &str) -> Result<(PathBuf, File)> {
    let time_nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for _ in 0..64 {
        let counter = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{file_name}.{}.{}.{}.tmp",
            std::process::id(),
            time_nonce,
            counter
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create atomic temp {}: {error}", path.display())),
        }
    }
    Err("could not allocate a unique atomic-write temporary file".into())
}

#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(format!(
            "atomic MoveFileExW failed: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target).map_err(|error| {
        format!(
            "atomic rename {} -> {} failed: {error}",
            source.display(),
            target.display()
        )
    })?;
    let parent = target
        .parent()
        .ok_or("atomic target has no parent directory after rename")?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "flush atomic target directory {}: {error}",
                parent.display()
            )
        })
}

fn enumerate_package_files(root: &Path) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    let mut directory_count = 1usize;
    while let Some((directory, prefix)) = stack.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("read package directory {}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("enumerate package directory: {error}"))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "package contains a non-UTF-8 path".to_string())?;
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            validate_package_path(&relative)?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("inspect package path {relative:?}: {error}"))?;
            if metadata.file_type().is_symlink() || is_windows_reparse_point(&entry.path())? {
                return Err(format!(
                    "package path {relative:?} is a symlink/reparse alias"
                ));
            }
            if metadata.is_dir() {
                directory_count = directory_count
                    .checked_add(1)
                    .ok_or("package directory count overflow")?;
                if directory_count > MAX_PACKAGE_DIRECTORIES {
                    return Err(format!(
                        "package contains more than {MAX_PACKAGE_DIRECTORIES} directories"
                    ));
                }
                stack.push((entry.path(), relative));
            } else if metadata.is_file() {
                if !paths.insert(relative.clone()) {
                    return Err(format!("duplicate package path {relative:?}"));
                }
                if paths.len() > MAX_PACKAGE_FILES + 1 {
                    return Err("package contains too many files".into());
                }
            } else {
                return Err(format!("package path {relative:?} is not a regular file"));
            }
        }
    }
    Ok(paths)
}

fn checked_join(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_package_path(relative)?;
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        path.push(component);
        refuse_symlink(&path, "package path component")?;
        let canonical = fs::canonicalize(&path).map_err(|error| {
            format!(
                "canonicalize package path component {}: {error}",
                path.display()
            )
        })?;
        if !canonical.starts_with(root) {
            return Err(format!(
                "package path component escapes the admitted root: {}",
                path.display()
            ));
        }
    }
    Ok(path)
}

fn hash_file(path: &Path) -> Result<(u64, String)> {
    let mut file = File::open(path)
        .map_err(|error| format!("open package file {}: {error}", path.display()))?;
    refuse_multi_link_file(&file, path)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read package file {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or("package file byte count overflow")?;
        if bytes > MAX_PACKAGE_BYTES {
            return Err(format!("package file {} exceeds size cap", path.display()));
        }
        hasher.update(&buffer[..read]);
    }
    Ok((bytes, format!("{:x}", hasher.finalize())))
}

#[cfg(windows)]
fn refuse_multi_link_file(file: &File, path: &Path) -> Result<()> {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(
            file: *mut c_void,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }

    let mut information = MaybeUninit::<ByHandleFileInformation>::uninit();
    let ok = unsafe {
        GetFileInformationByHandle(file.as_raw_handle().cast(), information.as_mut_ptr())
    };
    if ok == 0 {
        return Err(format!(
            "query package hard-link count for {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let links = unsafe { information.assume_init() }.number_of_links;
    if links != 1 {
        return Err(format!(
            "package file has {links} hard links and is not immutable-by-path: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn refuse_multi_link_file(file: &File, path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let links = file
        .metadata()
        .map_err(|error| format!("inspect open package file {}: {error}", path.display()))?
        .nlink();
    if links != 1 {
        return Err(format!(
            "package file has {links} hard links and is not immutable-by-path: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn refuse_multi_link_file(_file: &File, _path: &Path) -> Result<()> {
    Ok(())
}

fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    refuse_multi_link_file(&file, path)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect open file {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > cap {
        return Err(format!(
            "file {} is absent/non-regular or exceeds cap",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if bytes.len() as u64 > cap {
        return Err(format!("file {} grew beyond its read cap", path.display()));
    }
    Ok(bytes)
}

fn refuse_symlink(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || is_windows_reparse_point(path)? {
        return Err(format!(
            "{label} is a symlink/reparse alias: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_reparse_point(path: &Path) -> Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    let path = path
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == u32::MAX {
        return Err(format!(
            "query Windows package path attributes failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_path: &Path) -> Result<bool> {
    Ok(false)
}

fn validate_package_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 240
        || !path.is_ascii()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.split('/').any(|part| {
            part.is_empty()
                || part.len() > 128
                || part == "."
                || part == ".."
                || part.ends_with(' ')
                || part.ends_with('.')
                || is_windows_reserved_component(part)
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
    {
        return Err(format!("unsafe/non-portable package path {path:?}"));
    }
    Ok(())
}

fn is_windows_reserved_component(component: &str) -> bool {
    let stem = component
        .split_once('.')
        .map_or(component, |(value, _)| value)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem.strip_prefix("COM").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || stem.strip_prefix("LPT").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
}

fn validate_token(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!("{label} is not a portable token: {value:?}"));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not lowercase SHA-256"));
    }
    Ok(())
}

fn decode_lower_hex(value: &str, label: &str) -> Result<Vec<u8>> {
    if value.is_empty()
        || value.len() % 2 != 0
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not nonempty lowercase hexadecimal"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte: u8| -> Result<u8> {
                match byte {
                    b'0'..=b'9' => Ok(byte - b'0'),
                    b'a'..=b'f' => Ok(byte - b'a' + 10),
                    _ => Err(format!("{label} contains a non-hexadecimal byte")),
                }
            };
            Ok::<u8, String>((nibble(pair[0])? << 4) | nibble(pair[1])?)
        })
        .collect()
}

fn decode_sha256(value: &str, label: &str) -> Result<[u8; 32]> {
    validate_sha256(value, label)?;
    decode_lower_hex(value, label)?
        .try_into()
        .map_err(|_| format!("{label} does not decode to 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct FixtureVerifier;

    struct FixtureReleasePolicy;

    struct RefuseAllReleases;

    struct RefuseProcessCreation;

    struct PointerMutatingVerifier {
        install_root: PathBuf,
        mutated: Cell<bool>,
    }

    #[derive(Default)]
    struct FixtureEntrypointLauncher {
        calls: RefCell<Vec<(PathBuf, PathBuf, Vec<OsString>)>>,
    }

    impl AdmittedEntrypointLauncher for FixtureEntrypointLauncher {
        type Output = usize;

        fn launch(
            &self,
            entrypoint: &Path,
            working_directory: &Path,
            forwarded_args: &[OsString],
        ) -> Result<Self::Output> {
            self.calls.borrow_mut().push((
                entrypoint.to_path_buf(),
                working_directory.to_path_buf(),
                forwarded_args.to_vec(),
            ));
            Ok(forwarded_args.len())
        }
    }

    impl AdmittedEntrypointLauncher for RefuseProcessCreation {
        type Output = ();

        fn launch(
            &self,
            _entrypoint: &Path,
            _working_directory: &Path,
            _forwarded_args: &[OsString],
        ) -> Result<Self::Output> {
            Err("fixture process creation refusal".into())
        }
    }

    impl ReleaseAdmissionPolicy for FixtureReleasePolicy {
        fn admit_active(&self, active: &PackageManifest) -> Result<()> {
            if active.release_version.starts_with("1.") {
                Ok(())
            } else {
                Err("fixture active channel refused".into())
            }
        }

        fn admit_promotion(
            &self,
            current: Option<&PackageManifest>,
            candidate: &PackageManifest,
        ) -> Result<()> {
            if current.is_some_and(|value| {
                value.release_version.as_str() >= candidate.release_version.as_str()
            }) {
                Err("fixture downgrade/repeat refused".into())
            } else {
                Ok(())
            }
        }

        fn admit_rollback(
            &self,
            current: &PackageManifest,
            previous: &PackageManifest,
        ) -> Result<()> {
            if current.release_version.starts_with("1.")
                && previous.release_version.starts_with("1.")
                && current.release_version != previous.release_version
            {
                Ok(())
            } else {
                Err("fixture rollback refused".into())
            }
        }
    }

    impl ReleaseAdmissionPolicy for RefuseAllReleases {
        fn admit_active(&self, _active: &PackageManifest) -> Result<()> {
            Err("fixture release policy refuses active package".into())
        }

        fn admit_promotion(
            &self,
            _current: Option<&PackageManifest>,
            _candidate: &PackageManifest,
        ) -> Result<()> {
            Err("fixture release policy refuses promotion".into())
        }

        fn admit_rollback(
            &self,
            _current: &PackageManifest,
            _previous: &PackageManifest,
        ) -> Result<()> {
            Err("fixture release policy refuses rollback".into())
        }
    }

    impl PackageSignatureVerifier for FixtureVerifier {
        fn verify(
            &self,
            algorithm: &str,
            signer_id: &str,
            payload_sha256: &[u8; 32],
            signature: &[u8],
        ) -> Result<()> {
            if algorithm == "fixture-ed25519"
                && signer_id == "fixture-release"
                && signature == &Sha256::digest(&payload_sha256[..])[..]
            {
                Ok(())
            } else {
                Err("fixture signature mismatch".into())
            }
        }
    }

    impl PackageSignatureVerifier for PointerMutatingVerifier {
        fn verify(
            &self,
            algorithm: &str,
            signer_id: &str,
            payload_sha256: &[u8; 32],
            signature: &[u8],
        ) -> Result<()> {
            FixtureVerifier.verify(algorithm, signer_id, payload_sha256, signature)?;
            if !self.mutated.replace(true) {
                let mut pointer = load_active_pointer(&self.install_root)?
                    .ok_or("fixture active pointer disappeared")?;
                pointer.generation = pointer
                    .generation
                    .checked_add(1)
                    .ok_or("fixture generation overflow")?;
                fs::write(
                    self.install_root.join(ACTIVE_RELEASE_FILE),
                    serde_json::to_vec(&pointer)
                        .map_err(|error| format!("encode fixture pointer: {error}"))?,
                )
                .map_err(|error| format!("mutate fixture pointer: {error}"))?;
            }
            Ok(())
        }
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "replay-package-runtime-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn identity() -> RuntimeIdentity {
        RuntimeIdentity {
            product_id: PRODUCT_ID.into(),
            target: "fixture-x86_64".into(),
        }
    }

    fn production_policy() -> SigningPolicy {
        SigningPolicy::production(BTreeMap::from([(
            "fixture-ed25519".into(),
            BTreeSet::from(["fixture-release".into()]),
        )]))
        .unwrap()
    }

    fn write_release(root: &Path, version: &str, signed: bool) -> PackageManifest {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join("replay_app.exe"), version.as_bytes()).unwrap();
        fs::write(root.join("NOTICE.txt"), b"fixture notice").unwrap();
        // `build_unsigned_manifest` hashes both files that were just written.
        let guards = [root.join("replay_app.exe"), root.join("NOTICE.txt")];
        let guards = guards.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        let mut manifest = portable_test_fs_retry::with_read_guards(&guards, || {
            build_unsigned_manifest(root, version, "fixture-x86_64", "replay_app.exe")
        })
        .expect("hold the release payload open across the scanner's window")
        .unwrap();
        if signed {
            let payload =
                decode_sha256(&manifest.package_digest_sha256, "fixture package digest").unwrap();
            let signature = Sha256::digest(payload);
            manifest.signature = Some(PackageSignature {
                algorithm: "fixture-ed25519".into(),
                signer_id: "fixture-release".into(),
                payload_sha256: manifest.package_digest_sha256.clone(),
                signature_hex: format!("{signature:x}"),
            });
        }
        fs::write(
            root.join(PACKAGE_MANIFEST_FILE),
            manifest.canonical_bytes().unwrap(),
        )
        .unwrap();
        manifest
    }

    #[test]
    fn canonical_manifest_and_path_rules_are_deterministic() {
        let root = temp("canonical");
        let manifest = write_release(&root, "1.0.0", false);
        let bytes = manifest.canonical_bytes().unwrap();
        assert_eq!(PackageManifest::parse_canonical(&bytes).unwrap(), manifest);
        assert_eq!(
            manifest
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["NOTICE.txt", "replay_app.exe"]
        );
        assert!(
            build_unsigned_manifest(&root, "1.0.0", "fixture-x86_64", "replay_app.exe").is_err()
        );
        let mut padded = bytes.clone();
        padded.push(b'\n');
        assert!(PackageManifest::parse_canonical(&padded).is_err());
        assert!(validate_package_path("../escape.exe").is_err());
        assert!(validate_package_path("bin\\replay.exe").is_err());
        assert!(validate_package_path("bin/r\u{e9}play.exe").is_err());
        assert!(validate_package_path("bin/CON.txt").is_err());
        assert!(validate_package_path("aux/replay.exe").is_err());
        assert!(validate_package_path("Lpt9.dll").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_signing_fails_closed_without_every_policy_component() {
        let root = temp("signing");
        let unsigned_root = root.join("unsigned");
        let signed_root = root.join("signed");
        let unsigned = write_release(&unsigned_root, "1.0.0", false);
        assert!(admit_signature(&unsigned, &production_policy(), Some(&FixtureVerifier)).is_err());
        assert!(admit_signature(&unsigned, &SigningPolicy::unsigned_development(), None).is_ok());

        let signed = write_release(&signed_root, "1.0.1", true);
        assert!(admit_signature(&signed, &production_policy(), None).is_err());
        admit_signature(&signed, &production_policy(), Some(&FixtureVerifier)).unwrap();
        assert!(admit_signature(&signed, &SigningPolicy::unsigned_development(), None).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_verification_rejects_tamper_and_unlisted_files() {
        let root = temp("coverage");
        write_release(&root, "1.0.0", true);
        verify_release_dir(
            &root,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
        )
        .unwrap();
        fs::write(root.join("replay_app.exe"), b"tampered").unwrap();
        assert!(verify_release_dir(
            &root,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier)
        )
        .is_err());
        fs::write(root.join("replay_app.exe"), b"1.0.0").unwrap();
        fs::write(root.join("unlisted.dll"), b"no").unwrap();
        assert!(verify_release_dir(
            &root,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier)
        )
        .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn retention_copies_reverifies_and_never_activates_implicitly() {
        let root = temp("retain");
        let staged = root.join("staged");
        let install = root.join("installed");
        let manifest = write_release(&staged, "1.0.0", true);

        let first = retain_staged_release(
            &install,
            &staged,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
        )
        .unwrap();
        assert!(!first.already_present);
        assert_eq!(
            first.release.root,
            fs::canonicalize(
                install
                    .join(VERSIONS_DIR)
                    .join(&manifest.package_digest_sha256)
            )
            .unwrap()
        );
        assert!(!install.join(ACTIVE_RELEASE_FILE).exists());

        let second = retain_staged_release(
            &install,
            &staged,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
        )
        .unwrap();
        assert!(second.already_present);
        assert_eq!(second.release.manifest, manifest);

        let pointer = promote_release(
            &install,
            &manifest.package_digest_sha256,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
        )
        .unwrap();
        assert_eq!(pointer.current_digest_sha256, manifest.package_digest_sha256);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn retention_refuses_changed_source_and_corrupt_existing_digest_directory() {
        let root = temp("retain-refuse");
        let staged = root.join("staged");
        let install = root.join("installed");
        let manifest = write_release(&staged, "1.0.0", true);
        fs::write(staged.join("replay_app.exe"), b"changed").unwrap();
        assert!(retain_staged_release(
            &install,
            &staged,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
        )
        .is_err());
        assert!(!install.exists());

        fs::write(staged.join("replay_app.exe"), b"1.0.0").unwrap();
        let occupied = install
            .join(VERSIONS_DIR)
            .join(&manifest.package_digest_sha256);
        fs::create_dir_all(&occupied).unwrap();
        fs::write(occupied.join("untrusted.exe"), b"do not replace").unwrap();
        assert!(retain_staged_release(
            &install,
            &staged,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
        )
        .is_err());
        assert_eq!(
            portable_test_fs_retry::read(occupied.join("untrusted.exe")).unwrap(),
            b"do not replace"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_builder_refuses_hard_link_aliases() {
        let root = temp("hard-link");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("replay_app.exe"), b"fixture").unwrap();
        fs::hard_link(root.join("replay_app.exe"), root.join("alias.exe")).unwrap();
        // The guards matter MORE for a negative test than a positive one: this
        // asserts only `is_err()`, so a scanner's `os error 32` would satisfy it
        // and the hard-link refusal it exists to prove would go unproven.
        let _scanner_guards = portable_test_fs_retry::tree_read_guards(&root)
            .expect("hold the release payload open across the scanner's window");
        assert!(build_unsigned_manifest(
            &root,
            "1.0.0",
            "fixture-x86_64",
            "replay_app.exe"
        )
        .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn promotion_and_rollback_switch_only_verified_version_directories() {
        let install = temp("promotion");
        fs::create_dir_all(install.join(VERSIONS_DIR)).unwrap();
        let scratch_a = install.join("a");
        let manifest_a = write_release(&scratch_a, "1.0.0", true);
        let release_a = install
            .join(VERSIONS_DIR)
            .join(&manifest_a.package_digest_sha256);
        fs::rename(&scratch_a, &release_a).unwrap();
        let first = promote_release(
            &install,
            &manifest_a.package_digest_sha256,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
        )
        .unwrap();
        assert_eq!(first.generation, 1);
        assert!(first.previous_digest_sha256.is_none());

        let scratch_b = install.join("b");
        let manifest_b = write_release(&scratch_b, "1.1.0", true);
        let release_b = install
            .join(VERSIONS_DIR)
            .join(&manifest_b.package_digest_sha256);
        fs::rename(&scratch_b, &release_b).unwrap();
        let second = promote_release(
            &install,
            &manifest_b.package_digest_sha256,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
        )
        .unwrap();
        assert_eq!(
            second.previous_digest_sha256,
            Some(manifest_a.package_digest_sha256.clone())
        );
        assert_eq!(second.generation, 2);
        let active = discover_active_release(
            &install,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
        )
        .unwrap();
        assert_eq!(active.manifest.release_version, "1.1.0");
        assert!(discover_active_release(
            &install,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &RefuseAllReleases,
        )
        .is_err());
        assert!(promote_release(
            &install,
            &manifest_b.package_digest_sha256,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &RefuseAllReleases,
        )
        .is_err());
        assert!(rollback_release(
            &install,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &RefuseAllReleases,
        )
        .is_err());
        assert_eq!(
            load_active_pointer(&install)
                .unwrap()
                .unwrap()
                .current_digest_sha256,
            manifest_b.package_digest_sha256
        );

        let rolled_back = rollback_release(
            &install,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
        )
        .unwrap();
        assert_eq!(
            rolled_back.current_digest_sha256,
            manifest_a.package_digest_sha256
        );
        assert_eq!(
            rolled_back.previous_digest_sha256,
            Some(manifest_b.package_digest_sha256)
        );
        assert_eq!(rolled_back.generation, 3);
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn stable_launcher_invokes_only_the_admitted_entrypoint_and_forwards_args() {
        let install = temp("launcher");
        fs::create_dir_all(install.join(VERSIONS_DIR)).unwrap();
        let stable_launcher = install.join(STABLE_LAUNCHER_FILE);
        fs::write(&stable_launcher, b"fixture stable launcher").unwrap();

        let scratch = install.join("staged");
        let manifest = write_release(&scratch, "1.2.0", true);
        let release_root = install
            .join(VERSIONS_DIR)
            .join(&manifest.package_digest_sha256);
        fs::rename(&scratch, &release_root).unwrap();
        promote_release(
            &install,
            &manifest.package_digest_sha256,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
        )
        .unwrap();

        let forwarded_args = [
            OsString::from("--recording"),
            OsString::from("fixture recording.battleplan"),
        ];
        let process_launcher = FixtureEntrypointLauncher::default();
        let output = invoke_active_release_from_launcher_path(
            &stable_launcher,
            &forwarded_args,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
            &process_launcher,
        )
        .unwrap();
        assert_eq!(output, forwarded_args.len());
        let calls = process_launcher.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            fs::canonicalize(release_root.join("replay_app.exe")).unwrap()
        );
        assert_eq!(calls[0].1, fs::canonicalize(&release_root).unwrap());
        assert_eq!(calls[0].2, forwarded_args.to_vec());
        drop(calls);

        let preflight_error = invoke_active_release_from_launcher_path(
            &stable_launcher,
            &forwarded_args,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &RefuseAllReleases,
            &process_launcher,
        )
        .expect_err("release-policy failure must refuse before process creation");
        assert!(matches!(preflight_error, LauncherFailure::Preflight(_)));
        assert_eq!(process_launcher.calls.borrow().len(), 1);

        let process_error = invoke_active_release_from_launcher_path(
            &stable_launcher,
            &forwarded_args,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
            &RefuseProcessCreation,
        )
        .expect_err("process callback failure must remain distinguishable");
        assert_eq!(
            process_error,
            LauncherFailure::ProcessCreation("fixture process creation refusal".into())
        );

        let renamed_launcher = install.join("untrusted-name.exe");
        fs::write(&renamed_launcher, b"fixture renamed launcher").unwrap();
        assert!(install_root_from_launcher(&renamed_launcher).is_err());
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn active_pointer_change_during_verification_refuses_stale_launch() {
        let install = temp("active-pointer-race");
        fs::create_dir_all(install.join(VERSIONS_DIR)).unwrap();
        let scratch = install.join("staged");
        let manifest = write_release(&scratch, "1.3.0", true);
        let release_root = install
            .join(VERSIONS_DIR)
            .join(&manifest.package_digest_sha256);
        fs::rename(&scratch, release_root).unwrap();
        promote_release(
            &install,
            &manifest.package_digest_sha256,
            &identity(),
            &production_policy(),
            Some(&FixtureVerifier),
            &FixtureReleasePolicy,
        )
        .unwrap();

        let verifier = PointerMutatingVerifier {
            install_root: install.clone(),
            mutated: Cell::new(false),
        };
        let error = discover_active_release(
            &install,
            &identity(),
            &production_policy(),
            Some(&verifier),
            &FixtureReleasePolicy,
        )
        .expect_err("pointer race must refuse launch");
        assert!(error.contains("active release pointer changed during verification"));
        let _ = fs::remove_dir_all(install);
    }
}
