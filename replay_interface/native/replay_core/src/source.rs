//! Ingest boundary: recording bytes -> raw per-hour frames.
//!
//! The only place recording bytes enter the replay layer
//! (REPLAY_ARCHITECTURE.md, "Ingest boundary"). Backend 1 — the
//! snapshot-flavor `.battleplan` container or directory store — is read
//! through the verified `snapshot` crate reader (reuse, not a fork).
//! Backend 2 — a compact-stream-only container — remains rejected by the
//! ordinary product open path. An explicit caller may inject either neutral
//! certified capability; the production-shaped route authenticates to a
//! private spool through [`snapshot::CertifiedPrefoldSpoolDecoder`] and lazily
//! loads one immutable certified hour at a time.

use serde_json::{Map, Value};
use snapshot::{ExpectedIdentity, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Timeline format id the production recorder writes for compact-stream-only
/// recordings (`extractor/native/recorder_host`).
const COMPACT_STREAM_TIMELINE_FORMAT: &str = "hoi4_compact_stream_timeline_v1";
/// Manifest-entry / timeline role of the raw compact domain stream.
const COMPACT_STREAM_ROLE: &str = snapshot::COMPACT_STREAM_ROLE;
/// Shipped-player compact expansion ceiling. Two GiB is intentionally aligned
/// with the format's ordinary bounded-entry tier and is small enough for weak
/// offline PCs to reject pathological recordings before reserving product
/// spool storage. The streaming writer enforces this same limit again.
pub const PRODUCT_COMPACT_STREAM_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

fn admit_product_compact_stream_bytes(bytes: u64) -> Result<(), SourceError> {
    if bytes > PRODUCT_COMPACT_STREAM_MAX_BYTES {
        return Err(SourceError::Unsupported(format!(
            "compact replay expands to {bytes} bytes; shipped playback limit is {} bytes (2 GiB)",
            PRODUCT_COMPACT_STREAM_MAX_BYTES
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecorderSessionBinding {
    active_dlcs: Vec<String>,
    localisation_language: String,
    display_variant: RecorderDisplayVariant,
    campaign_identity: Option<RecordedCampaignIdentity>,
    product_admission: RecorderProductAdmission,
}

/// One guarantee of the recorder's `--profile production` capture profile that
/// an admitted recording does not carry.
///
/// A recording that holds all of them is a full production-profile capture. A
/// recording that holds fewer is still playable — but every consumer of
/// [`ReplaySource::product_admission`] is told exactly which guarantee is
/// missing and, through [`RecorderProductAdmission`], exactly what it covered.
///
/// WHY THIS ENUM EXISTS AT ALL. `--profile production` is early-launch-only:
/// its finalizer builds the portable content manifest from the effective-VFS
/// content observation authenticated in-process before `ResumeThread`, so it
/// needs the in-memory early-launch manifest and the authenticated early
/// process receipt, neither of which a `--pid` attach has. The shipped product
/// is a late attach and therefore deliberately does NOT run it —
/// `recorder_host/src/main.rs::product_auto_attach_worker_args`: "Do not pass
/// `--profile production` here. That profile additionally requires a
/// process-bound, pre-injection static bundle. The ordinary late-attach product
/// cannot fabricate or rebind that evidence." Refusing every recording whose
/// `production_profile.enabled` is not `true` therefore refuses every recording
/// the shipped attach-and-play product can make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProductionProfileGuarantee {
    /// The capture ran under `--profile production` / `--production`: the
    /// recorder unioned the whole production domain roster into the schedule,
    /// admitted and finalized a process-bound static bundle, and built the
    /// portable content manifest from the pre-`ResumeThread` effective-VFS
    /// observation.
    ///
    /// Absent on every late-attach capture. What replay loses with it is
    /// covered by the two guarantees below plus
    /// [`ReplaySource::content_manifest`], which already answers `None` for a
    /// recording that carries no portable content manifest.
    ProductionProfileEnabled,
    /// Every production-profile domain was scheduled, so the recording is
    /// expected to carry all of them.
    ///
    /// What "not held" NAMES depends on the authority, because this guarantee
    /// is manifest-only — the package cannot answer it. For a stated authority
    /// the domains in
    /// [`RecorderProductAdmission::profile_domains_not_scheduled`] were never
    /// captured and will read `DomainAbsent` at every hour. For
    /// [`ProductionProfileAuthority::Unstated`] the schedule is UNKNOWN and
    /// that list is empty because nothing can be named — an empty list there is
    /// unavailable evidence, never measured completeness.
    FullProfileDomainSchedule,
    /// Every required deterministic static artifact was carried. Not held means
    /// the artifacts named by
    /// [`RecorderProductAdmission::missing_required_static_artifacts`] are
    /// absent from the package; [`ReplaySource::static_artifact`] answers
    /// `None` for each, and replay never fabricates one.
    RequiredStaticArtifacts,
    /// The static artifacts the package DOES carry are bound to a finalized
    /// portable content-manifest epoch, so each catalog is joined to the exact
    /// effective content it was derived from.
    ///
    /// Not held means the package carries static artifacts that name no epoch
    /// and a container that finalized none — the production finalizer is what
    /// builds that manifest, from the pre-`ResumeThread` effective-VFS
    /// observation a late attach never made. Replay REFUSES those artifacts
    /// rather than joining an unbound catalog: they are named by
    /// [`RecorderProductAdmission::static_artifacts_not_content_bound`] and
    /// [`ReplaySource::static_artifact`] answers `None` for each.
    ContentBoundStaticArtifacts,
}

impl ProductionProfileGuarantee {
    /// Stable token for reports and logs. Never parsed back.
    pub fn token(self) -> &'static str {
        match self {
            ProductionProfileGuarantee::ProductionProfileEnabled => "production_profile_enabled",
            ProductionProfileGuarantee::FullProfileDomainSchedule => {
                "full_profile_domain_schedule"
            }
            ProductionProfileGuarantee::RequiredStaticArtifacts => "required_static_artifacts",
            ProductionProfileGuarantee::ContentBoundStaticArtifacts => {
                "content_bound_static_artifacts"
            }
        }
    }
}

/// What a recording's recorder manifest SAYS about the production capture
/// profile, as three distinguishable states.
///
/// Held apart from the guarantee list because they are different questions.
/// "This capture states it did not run the profile" and "this recording states
/// nothing about the profile at all" both leave the MANIFEST-ONLY guarantees
/// unevidenced, but only the first tells you the recorder looked.
///
/// # Which guarantee comes from where
///
/// Two of the four are manifest-only, because nothing in the package can answer
/// them: [`ProductionProfileGuarantee::ProductionProfileEnabled`] and
/// [`ProductionProfileGuarantee::FullProfileDomainSchedule`]. Absent a
/// `production_profile` block, both are reported as not held — unevidenced, not
/// disproved.
///
/// The other two are decided at the static-artifact roster join from the
/// VERIFIED PACKAGE, whatever the authority says and even when it says nothing:
/// [`ProductionProfileGuarantee::RequiredStaticArtifacts`] and
/// [`ProductionProfileGuarantee::ContentBoundStaticArtifacts`], together with
/// the two lists that name them. Where a declaration also exists it is
/// reconciled against the package rather than believed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionProfileAuthority {
    /// The recorder manifest carries no `production_profile` block. Nothing is
    /// CLAIMED, so both manifest-only guarantees are reported as not held —
    /// which is NOT the same as their being disproved — and
    /// [`RecorderProductAdmission::profile_domains_not_scheduled`] is empty
    /// because there is nothing to name. The two package-verifiable guarantees
    /// are still decided from the package, so an unstated recording carrying
    /// all eight epoch-bound artifacts HOLDS them.
    Unstated,
    /// `production_profile.enabled == false`. The capture states what it did
    /// and did not schedule, and `profile_domains_not_scheduled` is that
    /// statement. The static-artifact lists are the package's answer, which the
    /// capture's own declaration was reconciled against.
    Disabled,
    /// `production_profile.enabled == true`, and the recording satisfied every
    /// part of the profile ITS OWN RECORDER BUILD DEFINED — as far as replay
    /// can check.
    ///
    /// The static-artifact half is CHECKED AGAINST THE PACKAGE: an enabled
    /// profile whose package lacks an artifact its build required is refused at
    /// open. The schedule half is the manifest's DECLARATION, checked only in
    /// the one direction replay can compute without the extractor's domain
    /// registry — see `validate_recorder_product_admission`. An enabled
    /// manifest that under-declares its unscheduled profile domains is
    /// therefore admitted, and that limit is deliberate, named there, and
    /// closed producer-side.
    ///
    /// One package shortfall also survives, and it is not a shortfall against
    /// that build: a pre-focus-carrier package legitimately carries every
    /// artifact its build required, so it opens with the newer artifact
    /// reported missing.
    Enabled,
}

/// What a recording's recorder manifest positively claims about the production
/// capture profile, and — when it claims less than the whole profile — exactly
/// which guarantees it does not hold and what each of them covered.
///
/// This is the provenance side of the late-attach admission (owner ruling,
/// 2026-08-14). `open_recording` admits a capture that did not run the
/// production profile, and records the difference HERE rather than discarding
/// it: a consumer that needs the full profile can require
/// [`RecorderProductAdmission::is_full_production_profile`] and refuse for
/// itself, with the missing domains and artifacts already named.
///
/// It is not a loosening of the other admission authorities. A capture whose
/// manifest names a mandatory domain that is not production-complete is still
/// refused at open, enabled or not: that is a statement about the RECORDER
/// BUILD's producers rather than about the capture's scope, so no attach mode
/// explains it and the data it did deliver cannot be trusted as production
/// data. A capture that positively claims the production profile
/// (`enabled == true`) is still held to all of it — all of what ITS OWN
/// RECORDER BUILD required, which is the one qualification that keeps a
/// pre-focus-carrier production package readable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderProductAdmission {
    /// What the manifest said, so an absent authority is never read as a
    /// positive or a negative claim.
    pub authority: ProductionProfileAuthority,
    /// Production-profile guarantees this recording does not hold, ascending
    /// and deduplicated. Empty means a full production-profile capture.
    ///
    /// A guarantee is "not held" when the recording carries no positive
    /// evidence for it, which covers both "the capture said it fell short" and
    /// "the capture said nothing" — read [`RecorderProductAdmission::authority`]
    /// to tell those apart.
    pub guarantees_not_held: Vec<ProductionProfileGuarantee>,
    /// Production-profile domains the capture did not schedule, verbatim from
    /// `production_profile.profile_domains_not_scheduled`.
    ///
    /// EMPTY MEANS TWO DIFFERENT THINGS, and
    /// [`RecorderProductAdmission::authority`] is what separates them: for a
    /// stated authority it means the capture declared nothing unscheduled; for
    /// `Unstated` it means nothing was declared at all, and replay cannot
    /// derive the set from the package.
    pub profile_domains_not_scheduled: Vec<String>,
    /// Required static artifacts this recording does not carry, in canonical
    /// required-static-artifact registry order.
    ///
    /// Taken from the VERIFIED package once a recorder manifest is present, not
    /// from the manifest's declaration — the two are reconciled at the roster
    /// join and must agree, but the package is what `static_artifact()` answers
    /// from, and a legacy roster's declaration legitimately omits the one
    /// artifact its recorder build predates.
    pub missing_required_static_artifacts: Vec<String>,
    /// Static artifacts the package DOES carry, and replay refused, because
    /// neither they nor the container name a finalized portable
    /// content-manifest epoch to bind them to. In canonical
    /// required-static-artifact registry order, which is the order the roster
    /// join walks — not lexical.
    ///
    /// Filled at the roster join rather than from the manifest's profile
    /// block: it is a fact about the packaged bytes, not about the capture's
    /// declared scope.
    pub static_artifacts_not_content_bound: Vec<String>,
}

impl RecorderProductAdmission {
    /// Every guarantee held: the recording ran the whole production profile.
    pub fn is_full_production_profile(&self) -> bool {
        self.guarantees_not_held.is_empty()
    }

    /// The admission a recording that carries no `production_profile`
    /// authority at all presents.
    ///
    /// Every MANIFEST-ONLY guarantee is reported as not held, because neither
    /// is evidenced — an absent authority proves the schedule was complete
    /// exactly as much as it proves the profile ran, which is not at all.
    /// Naming only `ProductionProfileEnabled` here would have said the other
    /// WAS held (Codex round 1, MEDIUM).
    ///
    /// `RequiredStaticArtifacts` is deliberately NOT among them, and the split
    /// is the point: it is the one guarantee replay can VERIFY, because the
    /// package either carries the artifacts or does not. It is decided once, at
    /// the roster join, from what the package actually holds
    /// ([`MetadataRoute::admit_static_artifacts`]). Preloading it here made an
    /// unstated recording carrying all eight report the guarantee not held with
    /// nothing named missing (Codex round 8, MEDIUM). Which domains were
    /// scheduled, by contrast, is not visible in the package at all, so that
    /// one can only ever come from the manifest.
    fn unstated() -> Self {
        RecorderProductAdmission {
            authority: ProductionProfileAuthority::Unstated,
            guarantees_not_held: vec![
                ProductionProfileGuarantee::ProductionProfileEnabled,
                ProductionProfileGuarantee::FullProfileDomainSchedule,
            ],
            profile_domains_not_scheduled: Vec::new(),
            missing_required_static_artifacts: Vec::new(),
            static_artifacts_not_content_bound: Vec::new(),
        }
    }

    fn name_guarantee_not_held(&mut self, guarantee: ProductionProfileGuarantee) {
        self.guarantees_not_held.push(guarantee);
        self.guarantees_not_held.sort_unstable();
        self.guarantees_not_held.dedup();
    }
}

/// Shape-validated recorder-manifest campaign metadata retained by replay
/// ingest. This is not proof that `stream_header_sha256` matches CID1 bytes;
/// only the recorder's placement-authoritative stream path can prove that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCampaignIdentity {
    pub state: String,
    pub campaign_id: Option<String>,
    pub game_unique_id: Option<String>,
    pub game_unique_seed: Option<String>,
    pub source: Option<String>,
    pub source_build_profile: Option<String>,
    pub observed_game_hour_raw: Option<i32>,
    pub stream_header_sha256: Option<String>,
    pub resolution_note: Option<String>,
    pub witnesses: RecordedCampaignIdentityWitnesses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCampaignIdentityWitnesses {
    pub start_date_raw: Option<u32>,
    pub session: Option<u32>,
    pub scenario: Option<String>,
    pub initial_human_country_tags: Option<Vec<String>>,
    pub modset_identity: Option<String>,
}

/// Recorder-packaged effective decision catalog admitted by an exact
/// recorder-manifest -> replay-index -> container-manifest -> payload join.
///
/// The bytes are immutable and pointer-free. Replay consumers never reopen a
/// developer output path or an installed game tree to obtain this artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedDecisionCatalog {
    path: String,
    sha256: String,
    content_manifest_digest_sha256: String,
    bytes: Arc<[u8]>,
}

impl PackagedDecisionCatalog {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

/// One recorder-owned static catalog after the complete
/// recorder→index→container-role→content-epoch→bytes join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedStaticArtifact {
    name: String,
    path: String,
    sha256: String,
    content_manifest_digest_sha256: String,
    bytes: Arc<[u8]>,
}

impl PackagedStaticArtifact {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn content_manifest_digest_sha256(&self) -> &str {
        &self.content_manifest_digest_sha256
    }
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Author an artifact from loose bytes, WITHOUT the recorder→index→
    /// container-role→content-epoch join that every real one goes through.
    ///
    /// It exists so a consumer test can drive a semantic catalog admission
    /// (`static_research_catalog.rs`) over bytes it wrote itself, which is the
    /// only way to test a consumer against a catalog whose contents the test
    /// controls. The bytes it produces are unauthenticated by construction, so
    /// this must not be reachable from the product: it is behind the
    /// `test_support` feature, which only `game_ui`'s `[dev-dependencies]` turn
    /// on. Cargo's v2 resolver does not unify a dev-dependency feature into a
    /// normal build, so DEFAULT product resolution does not compile this at all
    /// — a stronger position than `#[doc(hidden)] pub`, which is always present.
    ///
    /// It is not an absolute exclusion, and should not be described as one: an
    /// explicit `--all-features` or an explicit feature selection turns it on in
    /// any profile. What holds then is only that no product source calls it,
    /// which is an ordinary review property rather than a compiler-enforced one.
    ///
    /// Everything downstream of it still runs for real: the semantic parse, the
    /// embedded self-hash, the cross-artifact content-epoch equality and every
    /// row-level rule are unchanged, so a test that hands it malformed bytes is
    /// refused exactly as a recording would be.
    #[cfg(feature = "test_support")]
    pub fn unauthenticated_for_consumer_tests(
        name: &str,
        path: &str,
        sha256: &str,
        content_manifest_digest_sha256: &str,
        bytes: Vec<u8>,
    ) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_string(),
            sha256: sha256.to_string(),
            content_manifest_digest_sha256: content_manifest_digest_sha256.to_string(),
            bytes: Arc::from(bytes.into_boxed_slice()),
        }
    }
}

/// The recorder's reserved stand-in for a manifest value the seal could not
/// make portable (`recorder_host::portable_manifest::PORTABILITY_EXCLUDED_VALUE`).
///
/// Spelled again here rather than imported: the extractor's crates are not
/// dependencies of replay, and what actually binds the two sides is the
/// container format, not a shared symbol. A container carrying this where
/// replay expects a claim has a HOLE there, accounted in the container's own
/// `portability_exclusions` ledger, and replay must refuse it rather than read
/// the stand-in as data.
const PORTABILITY_EXCLUDED_VALUE: &str = "__portability_excluded__";

/// The launch display variant, assembled from the settings binding.
///
/// The binding carries the display fields and the settings-file digest
/// SEPARATELY, because a recording holds two endpoint digests (attach and
/// capture close) and neither may be published as "the" settings hash. The
/// digest the display values were parsed from is the attach-time one, so that
/// is the one reassembled here.
fn recorder_display_variant_value(settings_binding: &Value) -> Option<Value> {
    let mut display = settings_binding.get("display_variant")?.as_object()?.clone();
    let attach = settings_binding.get("attach_settings_sha256")?.as_str()?;
    display.insert(
        "settings_file_sha256".to_string(),
        Value::String(attach.to_string()),
    );
    Some(Value::Object(display))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RecorderDisplayVariant {
    physical_width: u32,
    physical_height: u32,
    min_gui_width: u32,
    min_gui_height: u32,
    gui_scale_f32_bits: u32,
    refresh_rate_hz: u32,
    max_refresh_rate_hz: Option<u32>,
    fullscreen: bool,
    borderless: bool,
    display_mode: String,
    settings_file_sha256: String,
}

struct MetadataRoute {
    binding: Option<RecorderSessionBinding>,
    timeline: Option<Value>,
    recorder_manifest: Option<Map<String, Value>>,
}

impl MetadataRoute {
    /// Join the recorder-owned static-artifact roster and record, in this
    /// recording's provenance, every artifact the package carried and the join
    /// refused for want of a content-manifest epoch.
    ///
    /// The join is reached ONLY through here, so that it and the production
    /// profile claim it depends on are read from one object. A second reader of
    /// `production_profile.enabled` could drift from the one that produced the
    /// admission; this cannot. The refusal is recorded here rather than at the
    /// manifest's profile block because it is a fact about the packaged bytes
    /// rather than about the capture's declared scope.
    fn admit_static_artifacts(
        &mut self,
        store: &Store,
        game_id: &str,
    ) -> Result<BTreeMap<String, PackagedStaticArtifact>, SourceError> {
        let claims_production_profile = self.binding.as_ref().is_some_and(|binding| {
            binding.product_admission.authority == ProductionProfileAuthority::Enabled
        });
        let roster = validate_packaged_static_artifact_roster(
            store,
            game_id,
            self.recorder_manifest.as_ref(),
            claims_production_profile,
        )?;
        if let Some(binding) = self.binding.as_mut() {
            // RECONCILE THE DECLARATION AGAINST THE PACKAGE. The manifest's
            // `missing_required_static_artifacts` was believed until now, so a
            // capture could declare nothing missing while its index refused an
            // artifact — and the admission would report the static-artifact
            // guarantee as HELD while `static_artifact()` answered `None`
            // (Codex round 2, MEDIUM). A manifest that states nothing about the
            // profile (`Unstated`) declares no list, so there is nothing to
            // reconcile; the guarantee itself is decided below from the package,
            // exactly as it is for a stated authority.
            if binding.product_admission.authority != ProductionProfileAuthority::Unstated {
                let declared = &binding.product_admission.missing_required_static_artifacts;
                let declared_set = declared.iter().cloned().collect::<BTreeSet<_>>();
                if declared_set.len() != declared.len() {
                    return Err(SourceError::Open(format!(
                        "recorder production_profile.missing_required_static_artifacts repeats a \
                         name ({declared:?})"
                    )));
                }
                if let Some(unknown) = declared_set.iter().find(|name| {
                    !REQUIRED_PACKAGED_STATIC_ARTIFACTS
                        .iter()
                        .any(|(required, _)| required == name)
                }) {
                    return Err(SourceError::Open(format!(
                        "recorder production_profile.missing_required_static_artifacts names \
                         {unknown:?}, which is not a required static artifact"
                    )));
                }
                // THE WRITER'S ADVERTISED REQUIRED SET, VALIDATED FOR EVERY
                // STATED AUTHORITY. It used to be read only inside the legacy
                // branch, so a current eight-artifact package could advertise
                // `7`, `null`, repeats or unknown names and still open as a
                // full production profile (Codex round 12, MEDIUM). The
                // recorder writes this field unconditionally, so an absent or
                // malformed one is malformed, not legacy-compatible.
                let advertised = self
                    .recorder_manifest
                    .as_ref()
                    .and_then(|manifest| {
                        manifest
                            .get("production_profile")
                            .and_then(Value::as_object)
                            .and_then(|profile| profile.get("required_static_artifacts"))
                    })
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        SourceError::Open(
                            "recorder production_profile.required_static_artifacts is absent or \
                             is not an array, so what this recorder build required cannot be known"
                                .into(),
                        )
                    })?;
                let advertised_set = advertised
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        value.as_str().map(str::to_string).ok_or_else(|| {
                            SourceError::Open(format!(
                                "recorder production_profile.required_static_artifacts[{index}] \
                                 is not a string"
                            ))
                        })
                    })
                    .collect::<Result<BTreeSet<_>, _>>()?;
                if advertised_set.len() != advertised.len() {
                    return Err(SourceError::Open(
                        "recorder production_profile.required_static_artifacts repeats a name"
                            .into(),
                    ));
                }
                let canonical_current = REQUIRED_PACKAGED_STATIC_ARTIFACTS
                    .iter()
                    .map(|(name, _)| (*name).to_string())
                    .collect::<BTreeSet<_>>();
                // EQUALITY WITH A CANONICAL SET, not merely a plausible shape.
                // `[]`, unknown names, or a deliberately short roster would
                // otherwise read as "some other build", and inside the legacy
                // branch that meant "predates" — so a current build could
                // under-declare and pass as legacy (Codex round 5, MEDIUM).
                let mut declarable_absent = roster.absent.clone();
                match roster.legacy_skipped {
                    None => {
                        if advertised_set != canonical_current {
                            return Err(SourceError::Open(format!(
                                "recorder production_profile.required_static_artifacts \
                                 {advertised_set:?} is not the canonical required set \
                                 {canonical_current:?}"
                            )));
                        }
                    }
                    Some(skipped) => {
                        let canonical_legacy = canonical_current
                            .iter()
                            .filter(|name| name.as_str() != skipped)
                            .cloned()
                            .collect::<BTreeSet<_>>();
                        if advertised_set != canonical_legacy {
                            return Err(SourceError::Open(format!(
                                "static-artifact roster omits {skipped:?} but \
                                 production_profile.required_static_artifacts {advertised_set:?} \
                                 is not the canonical pre-{skipped} required set \
                                 {canonical_legacy:?}; only a recorder build that predates the \
                                 artifact is exempt from declaring it"
                            )));
                        }
                        declarable_absent.remove(skipped);
                    }
                }
                // A CAPTURE THAT CLAIMS THE PROFILE CARRIES EVERYTHING IT COULD
                // HAVE CARRIED. Rule 3 held the DECLARATION to an empty list;
                // the package is the stronger statement, and this closes the
                // shape where an enabled manifest declares nothing missing
                // while its rosters omit an artifact (Codex round 4, HIGH).
                //
                // It is asked of `declarable_absent`, AFTER the exemption is
                // established — not of `roster.absent`. A pre-focus production
                // package legitimately carries every artifact its build had,
                // and asking the raw set would refuse it for lacking one its
                // writer had never heard of, contradicting the readability this
                // join promises (Codex round 6, HIGH). The artifact is still
                // absent, so `RequiredStaticArtifacts` is still reported as not
                // held below.
                if binding.product_admission.authority == ProductionProfileAuthority::Enabled
                    && !declarable_absent.is_empty()
                {
                    return Err(SourceError::Open(format!(
                        "recorder production_profile.enabled is true but the verified package does \
                         not carry {declarable_absent:?}; a capture that claims the production \
                         profile carries every static artifact its recorder build required"
                    )));
                }
                if declared_set != declarable_absent {
                    return Err(SourceError::Open(format!(
                        "recorder production_profile.missing_required_static_artifacts \
                         {declared_set:?} disagrees with the artifacts the verified package does \
                         not carry {declarable_absent:?}"
                    )));
                }
            }
            // THE GUARANTEE AND THE NAMED LIST BOTH FOLLOW THE PACKAGE, NOT THE
            // DECLARATION, and they must follow the SAME thing or the provenance
            // contradicts itself.
            //
            // A legacy seven-row roster declares nothing missing and is right to
            // — its build had no eighth artifact — but the eighth is still
            // absent and `static_artifact()` still answers `None` for it, so the
            // guarantee is not held (Codex round 3, MEDIUM). Naming the
            // guarantee while leaving the list as the declaration's `[]` printed
            // "admitted without required_static_artifacts / required static
            // artifacts missing: none" (Codex round 7, MEDIUM). The list is
            // therefore replaced with what the VERIFIED package lacks, in
            // canonical registry order. The declaration is not discarded — it
            // was just proven equal to this set, minus any legacy exemption.
            //
            // Done outside the reconciliation block so an `Unstated` manifest,
            // which declares nothing to reconcile, still says what its package
            // lacks.
            binding.product_admission.missing_required_static_artifacts =
                REQUIRED_PACKAGED_STATIC_ARTIFACTS
                    .iter()
                    .map(|(name, _)| (*name).to_string())
                    .filter(|name| roster.absent.contains(name))
                    .collect();
            if !roster.absent.is_empty() {
                binding
                    .product_admission
                    .name_guarantee_not_held(ProductionProfileGuarantee::RequiredStaticArtifacts);
            }
            if !roster.not_content_bound.is_empty() {
                binding
                    .product_admission
                    .name_guarantee_not_held(ProductionProfileGuarantee::ContentBoundStaticArtifacts);
            }
            binding.product_admission.static_artifacts_not_content_bound =
                roster.not_content_bound;
        }
        Ok(roster.admitted)
    }
}

#[derive(Debug)]
pub enum SourceError {
    /// The recording could not be opened, read, or validated.
    Open(String),
    /// Retained bytes disagree with the digest/length that names them: a failed
    /// SHA-256, a failed CRC-32, a truncated/short payload, or a spooled object
    /// that changed underneath an authenticated read.
    ///
    /// A sibling of [`SourceError::Open`] (identical Display shape) typed so
    /// consumers can classify "these bytes are corrupt or substituted" apart
    /// from "this recording is malformed" without matching on error text
    /// (`SEAM_CONTRACTS.md`, ingest boundary). Integrity is a strictly stronger
    /// claim than malformation: it names bytes that were authenticated against a
    /// declared digest and lost.
    Integrity(String),
    /// The recording path does not exist. A sibling of [`SourceError::Open`]
    /// (identical Display shape) typed so consumers can classify "not found"
    /// without matching on error text.
    NotFound(String),
    /// The recording's manifest mod identity disagrees with its snapshot
    /// provenance ([`snapshot::ContainerError::ModsetMismatch`]), typed through
    /// so the product surface classifies a modset mismatch without string
    /// matching across crate boundaries.
    ModsetMismatch(String),
    /// The recording was recognized but this build cannot play it back.
    Unsupported(String),
    /// The recording was recognized as the compact-stream-only production
    /// flavor: an empty snapshot index alongside positive compact-stream
    /// evidence (a `collector_compact_stream` manifest role or a compact
    /// timeline declaration). A sibling of [`SourceError::Unsupported`]
    /// (identical Display shape) typed so consumers — the snapshot-native
    /// dump preflight in particular — can waive exactly this subclass
    /// without matching on error text.
    UnsupportedCompactStreamOnly(String),
    /// The store holds several games; pick one with [`open_recording_game`].
    MultipleGames(Vec<String>),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // NotFound/ModsetMismatch/Integrity keep the exact "open: ..."
            // rendering the untyped Open produced, so every message contract
            // holds verbatim.
            SourceError::Open(m)
            | SourceError::NotFound(m)
            | SourceError::ModsetMismatch(m)
            | SourceError::Integrity(m) => {
                write!(f, "open: {m}")
            }
            SourceError::Unsupported(m) | SourceError::UnsupportedCompactStreamOnly(m) => {
                write!(f, "unsupported recording: {m}")
            }
            SourceError::MultipleGames(games) => write!(
                f,
                "store holds {} games ({}); select one explicitly with open_recording_game",
                games.len(),
                games.join(", ")
            ),
        }
    }
}
impl std::error::Error for SourceError {}

/// True when a snapshot-layer load failure is a PAYLOAD-INTEGRITY failure —
/// retained bytes that disagree with the digest/length naming them — rather
/// than a malformed or semantically inconsistent recording.
///
/// Total match on the TYPED snapshot errors; no `.contains()` on error text, so
/// the wording of a portable message can change without silently reclassifying
/// a corrupt payload as a schema problem (or the reverse).
fn is_integrity_failure(error: &snapshot::LoadError) -> bool {
    let snapshot::LoadError::Container(container) = error else {
        return false;
    };
    is_container_integrity_failure(container)
}

/// The container-level half of [`is_integrity_failure`], for the many routes
/// that read a container entry directly instead of going through `Store`.
pub(crate) fn is_container_integrity_failure(container: &snapshot::ContainerError) -> bool {
    match container {
        snapshot::ContainerError::PayloadIntegrity(_) => true,
        // `Truncated` is NOT here: its call sites are structural shortfalls
        // (EOCD, central directory, Zip64 records), not payload bytes.
        snapshot::ContainerError::Zip(zip) => matches!(
            zip,
            snapshot::zip::ZipError::PayloadIntegrity(_)
                | snapshot::zip::ZipError::SizeMismatch { .. }
                | snapshot::zip::ZipError::Decompress(_)
        ),
        _ => false,
    }
}

/// Adapt a snapshot-layer load failure into the ingest error class it belongs
/// to, preserving the integrity class losslessly instead of flattening every
/// cause into [`SourceError::Open`].
fn source_error_from_load(error: &snapshot::LoadError, message: String) -> SourceError {
    match error {
        snapshot::LoadError::Container(snapshot::ContainerError::ModsetMismatch { .. }) => {
            SourceError::ModsetMismatch(message)
        }
        error if is_integrity_failure(error) => SourceError::Integrity(message),
        _ => SourceError::Open(message),
    }
}

/// As [`source_error_from_load`], for the routes that hold a
/// [`snapshot::Container`] and read/authenticate one entry directly. Every such
/// site must classify through here: an authenticated read that fails its digest
/// is the integrity class no matter which route performed it, and the shipped
/// product open path funnels through these same routes.
fn source_error_from_container(error: &snapshot::ContainerError, message: String) -> SourceError {
    match error {
        snapshot::ContainerError::ModsetMismatch { .. } => SourceError::ModsetMismatch(message),
        error if is_container_integrity_failure(error) => SourceError::Integrity(message),
        _ => SourceError::Open(message),
    }
}

/// One raw, schema-shaped frame (REPLAY_SCHEMA.md §1).
///
/// `hour_index` is the INDEX's hour for this frame — the index is the
/// canonical authority (§0.3). `rest` carries every other top-level domain
/// untouched, so unknown schema growth is preserved and reachable, never
/// dropped. This type deliberately does not expose `snapshot` crate types.
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub hour_index: i64,
    pub date: Option<String>,
    pub save_version: Option<String>,
    pub schema_version: Option<Value>,
    pub rest: Map<String, Value>,
}

/// One recorder-DECLARED main-timeline coverage gap, carried verbatim from the
/// recording index (`index.coverage`; `STITCHING_DESIGN.md` §7.1) across the
/// ingest boundary.
///
/// This is the recorder's own statement that a span of hours is missing and
/// WHY. It is never inferred from hour arithmetic: a downstream consumer that
/// reconstructs anonymous ranges from discontinuous hours has lost the reason,
/// the witness, and the owning segment. Per `documents/DECISIONS.md` a gap
/// carries its hour, its reason, the domains it affects, and a witness:
/// * hours — `start_hour..=end_hour`, inclusive, on the canonical hour axis;
/// * reason — `reason`, the recorder's reason text, plus the `kind` class;
/// * affected domains — an index coverage row is a WHOLE-HOUR absence, so every
///   domain is affected at every hour of the span (see
///   [`DeclaredCoverageGap::affects_all_domains`]); a coverage row never
///   describes a partial, per-domain loss;
/// * witness — `evidence_id`, plus `segment_id`/`owner` binding the claim to
///   the segment and observation that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredCoverageGap {
    /// First absent canonical hour (inclusive).
    pub start_hour: i64,
    /// Last absent canonical hour (inclusive); equals `start_hour` for a
    /// single-hour gap.
    pub end_hour: i64,
    /// Disposition class: `"gap"`, `"capture_failed"`, or `"coalesced"`. Never
    /// `"snapshot"` — a snapshot hour is playable, not a gap.
    pub kind: String,
    /// The recorder's reason text. Nonblank by admission.
    pub reason: String,
    /// The recorder's evidence id — the witness for this claim. Nonblank by
    /// admission.
    pub evidence_id: String,
    /// Owning segment, when the index names one.
    pub segment_id: Option<u32>,
    /// The selected-observation owner object, verbatim, when the index carries
    /// one. Opaque here: replay ingest never reinterprets recorder ownership.
    pub owner: Option<Value>,
    /// The recorder's selection evidence for this span, when it emitted any.
    /// Present as a bundle because the recorder emits its four members together
    /// or not at all (`add_main_ownership_fields` in the stitcher), so a
    /// partial bundle is a corrupt row rather than a smaller claim.
    pub selection: Option<DeclaredCoverageSelection>,
}

/// The exact observation the recorder SELECTED for a covered span, and the
/// decoded source extent that observation occupies.
///
/// This is the provenance that makes a gap auditable back to bytes: without it
/// a consumer has the reason and the evidence id but cannot point at the source
/// region the claim was derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredCoverageSelection {
    /// The recorder's same-hour selection rule id.
    pub selection_rule: String,
    /// The selected observation's id.
    pub selected_observation_id: String,
    /// Inclusive decoded record-sequence bounds of the selected observation.
    pub source_record_seq_bounds: (u64, u64),
    /// Half-open decoded byte bounds of the selected observation.
    pub source_byte_bounds: (u64, u64),
}

impl DeclaredCoverageGap {
    /// True when `hour` falls inside this declared gap.
    pub fn covers(&self, hour: i64) -> bool {
        self.start_hour <= hour && hour <= self.end_hour
    }
    /// Number of absent hours this row declares (always >= 1).
    pub fn hour_count(&self) -> u64 {
        self.end_hour.abs_diff(self.start_hour).saturating_add(1)
    }
    /// An index coverage row is a whole-hour absence: no frame exists for those
    /// hours at all, so every domain is affected. Present as a named predicate
    /// so a consumer never has to infer the domain scope from the row's shape.
    pub fn affects_all_domains(&self) -> bool {
        true
    }
}

/// The recording index's own declared coverage statement — the stitched-index
/// fields (`STITCHING_DESIGN.md` §7.1) that the ingest boundary must carry
/// instead of dropping.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeclaredCoverage {
    /// `index.index_schema` (`"stitched-main-v1"` for a stitched index).
    pub index_schema: Option<String>,
    /// `index.segments_file` — where the authoritative branch graph lives.
    pub segments_file: Option<String>,
    /// `index.main_head` (`{ segment_id, hour_index }`), verbatim.
    pub main_head: Option<Value>,
    /// Declared gaps, ascending by `start_hour` and non-overlapping.
    pub gaps: Vec<DeclaredCoverageGap>,
}

impl DeclaredCoverage {
    /// True when this index declares itself stitched.
    pub fn is_stitched(&self) -> bool {
        self.index_schema.as_deref() == Some(STITCHED_INDEX_SCHEMA)
    }
    /// The declared gap covering `hour`, if any.
    pub fn gap_at(&self, hour: i64) -> Option<&DeclaredCoverageGap> {
        self.gaps.iter().find(|gap| gap.covers(hour))
    }
    /// True when the index carries nothing beyond a plain (v1) snapshot list.
    pub fn is_empty(&self) -> bool {
        self.index_schema.is_none()
            && self.segments_file.is_none()
            && self.main_head.is_none()
            && self.gaps.is_empty()
    }
}

/// The stitched index schema id (`STITCHING_DESIGN.md` §7.1).
const STITCHED_INDEX_SCHEMA: &str = "stitched-main-v1";

/// The complete set of non-snapshot coverage dispositions the recorder emits
/// (`CoverageDisposition` in `recorder_host/src/stitching.rs`). Closed on
/// purpose: an unrecognized kind is a gap whose class no consumer can act on.
const DECLARED_COVERAGE_KINDS: &[&str] = &["gap", "capture_failed", "coalesced"];

/// Parse the recorder-declared coverage statement out of one recording index.
///
/// Fail-closed: a coverage row that cannot be read as a complete gap claim is
/// refused, never silently dropped. Dropping it would let a recording that
/// ANNOUNCES missing hours reach the fold as an unexplained hour discontinuity.
fn read_declared_coverage(
    game_id: &str,
    index: &snapshot::Index,
) -> Result<DeclaredCoverage, SourceError> {
    let is_stitched = index.is_stitched();
    let mut gaps: Vec<DeclaredCoverageGap> = Vec::with_capacity(index.coverage.len());
    for (ordinal, row) in index.coverage.iter().enumerate() {
        let context = format!("recording {game_id:?} index.coverage[{ordinal}]");
        let row = row.as_object().ok_or_else(|| {
            SourceError::Open(format!("{context} is not an object; refusing to drop it"))
        })?;
        let field_i64 = |name: &str| -> Result<i64, SourceError> {
            row.get(name).and_then(Value::as_i64).ok_or_else(|| {
                SourceError::Open(format!("{context}.{name} is missing or not an integer"))
            })
        };
        let field_text = |name: &str| -> Result<String, SourceError> {
            row.get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    SourceError::Open(format!("{context}.{name} is missing, blank, or not a string"))
                })
        };
        let field_u64_pair = |name: &str| -> Result<(u64, u64), SourceError> {
            let bad = || SourceError::Open(format!("{context}.{name} is not a [u64, u64] bound"));
            let values = row.get(name).and_then(Value::as_array).ok_or_else(bad)?;
            match values.as_slice() {
                [first, second] => Ok((
                    first.as_u64().ok_or_else(bad)?,
                    second.as_u64().ok_or_else(bad)?,
                )),
                _ => Err(bad()),
            }
        };
        let start_hour = field_i64("start_hour")?;
        let end_hour = field_i64("end_hour")?;
        if start_hour > end_hour {
            return Err(SourceError::Open(format!(
                "{context} spans {start_hour}..={end_hour} (end before start)"
            )));
        }
        let kind = field_text("kind")?;
        if kind == "snapshot" {
            return Err(SourceError::Open(format!(
                "{context} declares kind \"snapshot\"; a coverage row is an absence, not a frame"
            )));
        }
        // Closed set. The stitched-container validator enforces this for a
        // packed v2 container, but a directory store never reaches that
        // validator, so an unknown disposition would otherwise be carried into
        // playback as a gap whose class nothing downstream can interpret.
        if !DECLARED_COVERAGE_KINDS.contains(&kind.as_str()) {
            return Err(SourceError::Open(format!(
                "{context} declares unknown coverage kind {kind:?} (admitted kinds: {})",
                DECLARED_COVERAGE_KINDS.join(", ")
            )));
        }
        let segment_id = match row.get("segment_id") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| {
                        SourceError::Open(format!("{context}.segment_id is not a segment ordinal"))
                    })?,
            ),
        };
        let owner = match row.get("owner") {
            None | Some(Value::Null) => None,
            Some(value) if value.is_object() => Some(value.clone()),
            Some(_) => {
                return Err(SourceError::Open(format!(
                    "{context}.owner is present but is not an object"
                )))
            }
        };
        // The recorder emits the four selection-evidence members together or
        // not at all. Admit them the same way: a partial bundle means the row
        // was rewritten, and keeping the surviving half would present partial
        // provenance as if it were the whole claim.
        let selection_members = [
            "selection_rule",
            "selected_observation_id",
            "source_record_seq_bounds",
            "source_byte_bounds",
        ];
        let present = selection_members
            .iter()
            .filter(|name| row.get(**name).is_some_and(|value| !value.is_null()))
            .count();
        let selection = match present {
            0 => None,
            count if count == selection_members.len() => Some(DeclaredCoverageSelection {
                selection_rule: field_text("selection_rule")?,
                selected_observation_id: field_text("selected_observation_id")?,
                source_record_seq_bounds: field_u64_pair("source_record_seq_bounds")?,
                source_byte_bounds: field_u64_pair("source_byte_bounds")?,
            }),
            count => {
                return Err(SourceError::Open(format!(
                    "{context} carries {count} of {} selection-evidence members; a partial \
                     selection bundle is not admissible provenance",
                    selection_members.len()
                )))
            }
        };
        // A stitched row always names its owning segment and observation owner
        // (`add_main_ownership_fields` emits both unconditionally). It does NOT
        // always carry selection evidence: a cross-session TRANSITION gap has
        // no logical frame observation to select, so the recorder legitimately
        // emits the row without a bundle (`MainHour::selected_observation` is
        // `None` there). Requiring it would refuse real recordings.
        if is_stitched && (segment_id.is_none() || owner.is_none()) {
            return Err(SourceError::Open(format!(
                "{context} is a stitched-index coverage row but omits its owning segment or its \
                 observation owner; a stitched gap names who owns the hours it declares absent"
            )));
        }
        if let Some(previous) = gaps.last() {
            if start_hour <= previous.end_hour {
                return Err(SourceError::Open(format!(
                    "{context} starts at {start_hour}, at or before the previous declared gap end \
                     {} (coverage must be ascending and non-overlapping)",
                    previous.end_hour
                )));
            }
        }
        gaps.push(DeclaredCoverageGap {
            start_hour,
            end_hour,
            kind,
            reason: field_text("reason")?,
            evidence_id: field_text("evidence_id")?,
            segment_id,
            owner,
            selection,
        });
    }
    Ok(DeclaredCoverage {
        index_schema: index.index_schema.clone(),
        segments_file: index.segments_file.clone(),
        main_head: index.main_head.clone(),
        gaps,
    })
}

/// A recording backend. Portable production recordings carry canonical H4CR
/// plus a certified hourly offset index and sparse binary checkpoints. Legacy
/// and debug recordings may carry per-hour JSON snapshots.
pub trait ReplaySource: Send + Sync {
    fn game_id(&self) -> &str;
    /// Sorted, deduplicated canonical hour indexes, taken from the recording
    /// INDEX (the authority). Entries without an `hour_index` are skipped and
    /// reported in [`ReplaySource::warnings`], never guessed.
    fn hours(&self) -> &[i64];
    /// Non-fatal ingest findings (index entries skipped, duplicate hours, ...).
    fn warnings(&self) -> &[String];
    /// The recording index's own DECLARED coverage statement, when it carries
    /// one. `None` means the index declared nothing (a plain v1 index with no
    /// coverage rows), which is NOT the same as declaring full coverage.
    ///
    /// Consumers must prefer a declared gap over any hour-arithmetic inference:
    /// a declared gap names its reason, its witness, and its owning segment,
    /// and an inferred one cannot.
    fn declared_coverage(&self) -> Option<&DeclaredCoverage> {
        None
    }
    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        None
    }
    /// Internally consistent campaign metadata from the recorder manifest.
    /// This never authorizes stitching without the recorder's
    /// placement-authenticated CID1 path and exact retained payload bytes.
    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        None
    }
    /// What this recording claims about the recorder's production capture
    /// profile, and which of that profile's guarantees it does not hold.
    ///
    /// `None` means the recording carried no recorder manifest at all (a
    /// snapshot-store or already-prefolded source), which is a different
    /// statement from a manifest that carried no `production_profile`
    /// authority — the latter answers `Some` with
    /// [`ProductionProfileGuarantee::ProductionProfileEnabled`] named as not
    /// held.
    ///
    /// A consumer that requires a full production-profile capture checks
    /// [`RecorderProductAdmission::is_full_production_profile`] and refuses for
    /// itself. The missing static artifacts are always named, because the
    /// package is read. The missing DOMAINS are named only when the manifest
    /// stated them: an `Unstated` authority reports the schedule guarantee as
    /// not held with nothing named, because which domains were scheduled is not
    /// visible in the package and no claim was made.
    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        None
    }
    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        None
    }
    /// Exact effective decision catalog authenticated through the portable
    /// package's recorder, index, content-manifest, and entry hashes.
    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        None
    }
    fn static_artifact(&self, _name: &str) -> Option<&PackagedStaticArtifact> {
        None
    }
    /// Exact source-byte and decoder identity for a certified compact-stream
    /// source. Snapshot-native sources return `None`.
    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        None
    }
    /// Date metadata for one canonical hour, without decoding its frame body.
    ///
    /// Production sources answer this from their authenticated index or
    /// prefold manifest. Backends that do not carry indexed dates return
    /// `None`; callers must not force a frame load merely to populate
    /// timeline navigation metadata.
    fn date_at(&self, _hour_index: i64) -> Result<Option<String>, SourceError> {
        Ok(None)
    }
    /// Raw snapshot frame for one canonical hour from [`ReplaySource::hours`].
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError>;

    /// Install a stop switch this source's reconstructions may consult, so a
    /// caller that has moved on can abandon a walk that is still running.
    ///
    /// Default: ignore it. Only a source whose `frame` does bounded, resumable
    /// work worth abandoning implements this; the rest answer from data they
    /// already hold and there is nothing to stop. A source that ignores the
    /// switch is not wrong, only uninterruptible.
    fn set_scrub_interrupt(&self, _interrupt: Option<Arc<dyn Fn() -> bool + Send + Sync>>) {}
}

/// Open a recording path. A regular file is opened as a `.battleplan`
/// container; a directory is opened as a snapshot store root (a directory
/// containing `<game_id>/index.json`). A directory store holding more than
/// one game returns [`SourceError::MultipleGames`].
pub fn open_recording(path: &Path) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, None, None)
}

/// As [`open_recording`], selecting one game explicitly (required when a
/// directory store holds several games).
pub fn open_recording_game(
    path: &Path,
    game_id: &str,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), None, None)
}

/// Open a recording with an explicitly supplied portable certified pre-folder.
///
/// Snapshot-flavor recordings follow the ordinary path and do not invoke the
/// decoder. For a compact-stream-only `.battleplan`, the verified unique
/// `collector_compact_stream` entry is transferred to `decoder`; replay_core
/// admits only the complete [`snapshot::PrefoldedReplay`] it returns. The
/// default [`open_recording`] policy remains fail-closed and no extractor crate
/// is a dependency of replay_core.
pub fn open_recording_with_decoder(
    path: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, Some(decoder), None)
}

/// As [`open_recording_with_decoder`], selecting one game explicitly.
pub fn open_recording_game_with_decoder(
    path: &Path,
    game_id: &str,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), Some(decoder), None)
}

/// Open a compact-stream-only container through the bounded authenticated
/// spool seam. `spool_base` must be an existing regular directory; replay_core
/// creates a private identity-marked child, removes it on every failed open,
/// and retains it only for the lifetime of the returned lazy source.
pub fn open_recording_with_spooled_decoder(
    path: &Path,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, None, None, Some((spool_base, decoder)))
}

/// Consume an already-open pinned container through the bounded decoder seam.
/// No pathname is reopened: the resulting store and lazy source retain the
/// exact container handle authenticated by the caller.
pub fn open_container_with_spooled_decoder(
    container: snapshot::Container,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_store_impl(
        Store::from_container(container),
        "authenticated retained replay container",
        None,
        None,
        Some((spool_base, decoder)),
    )
}

/// As [`open_recording_with_spooled_decoder`], selecting one game explicitly.
pub fn open_recording_game_with_spooled_decoder(
    path: &Path,
    game_id: &str,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl(path, Some(game_id), None, Some((spool_base, decoder)))
}

/// Admit a neutral set of canonical snapshots that was fully folded before it
/// crossed into the standalone replay layer.
///
/// This is intentionally a different boundary from [`open_recording`]. It does
/// not accept H4CR bytes, a compact-stream cursor, or an extractor callback.
/// Compact-only packages therefore remain fail-closed until an extractor-side
/// producer has completed/certified the fold and supplied [`snapshot::PrefoldedReplay`].
/// This is a trusted-capability API for an already authenticated producer; use
/// [`open_recording_with_decoder`] when source-byte length/hash and decoder
/// identity must be bound mechanically by the portable invocation receipt.
pub fn open_prefolded_replay(
    replay: snapshot::PrefoldedReplay,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    validate_index_schema_version(Some(replay.index().schema_version()))?;
    Ok(Box::new(PrefoldedSource {
        replay,
        warnings: Vec::new(),
        store: None,
        recorder_binding: None,
        design_catalog: None,
        decision_catalog: None,
        static_artifacts: BTreeMap::new(),
        prefold_receipt: None,
    }))
}

fn open_recording_impl(
    path: &Path,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    open_recording_impl_with_post_anchor(path, requested_game, decoder, spool_decoder, |_| {})
}

fn open_recording_impl_with_post_anchor<F>(
    path: &Path,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
    post_anchor: F,
) -> Result<Box<dyn ReplaySource>, SourceError>
where
    F: FnOnce(&Store),
{
    // Acquire the object's metadata ONCE and classify from it. `is_file()` /
    // `is_dir()` both answer false for an existing-but-unreadable path (a
    // permission or IO failure), which would misreport a real recording as
    // absent; only a metadata error whose kind is NotFound is "not found".
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(SourceError::NotFound(format!(
                "path does not exist (expected a .battleplan file or a snapshot store directory): {}",
                path.display()
            )))
        }
        Err(error) => {
            return Err(SourceError::Open(format!(
                "inspect recording path {}: {error}",
                path.display()
            )))
        }
    };
    let store = if metadata.is_file() {
        Store::open_replay(path, &ExpectedIdentity::default()).map_err(|e| {
            // Preserve the snapshot layer's typed modset mismatch and typed
            // payload-integrity class across the adapter (no string matching).
            source_error_from_load(&e, format!("open .battleplan: {e}"))
        })?
    } else if metadata.is_dir() {
        Store::open_directory(path)
            .map_err(|e| source_error_from_load(&e, format!("anchor directory store: {e}")))?
    } else {
        // DELIBERATE message change, scoped to a case whose old rendering was
        // the defect: an existing-but-special object (a pipe, a device) used to
        // be reported as `NotFound("path does not exist ...")`, which is false.
        // The absent-path rendering above is unchanged, and it is the only one
        // any fixture or product surface pins.
        return Err(SourceError::Open(format!(
            "recording path is neither a .battleplan file nor a snapshot store directory: {}",
            path.display()
        )));
    };
    post_anchor(&store);

    open_store_impl(
        store,
        &path.display().to_string(),
        requested_game,
        decoder,
        spool_decoder,
    )
}

fn open_store_impl(
    store: Store,
    source_label: &str,
    requested_game: Option<&str>,
    decoder: Option<&dyn snapshot::CertifiedPrefoldDecoder>,
    spool_decoder: Option<(&Path, &dyn snapshot::CertifiedPrefoldSpoolDecoder)>,
) -> Result<Box<dyn ReplaySource>, SourceError> {

    let games = store
        .games()
        .map_err(|e| SourceError::Open(format!("list games in store: {e}")))?;
    let game_id = match requested_game {
        Some(g) => {
            if !games.iter().any(|have| have == g) {
                return Err(SourceError::Open(format!(
                    "game {g:?} not found in store (available: {})",
                    if games.is_empty() {
                        "none".to_string()
                    } else {
                        games.join(", ")
                    }
                )));
            }
            g.to_string()
        }
        None => match games.len() {
            0 => {
                return Err(SourceError::Open(format!(
                    "no game found (no <game_id>/index.json) under {source_label}"
                )))
            }
            1 => games[0].clone(),
            _ => return Err(SourceError::MultipleGames(games)),
        },
    };

    // Route recorder metadata before consulting snapshot availability. An
    // explicit or inconsistent metadata signal can never be downgraded to
    // legacy/unsupported because an index or role entry is missing.
    let mut metadata = recorder_metadata_route(&store, &game_id)?;
    let index = store
        .index(&game_id)
        .map_err(|e| source_error_from_load(&e, format!("read index for game {game_id:?}: {e}")))?;
    validate_recorder_portable_container_contract(&store, &game_id, &index, &metadata)?;

    // The recorder's DECLARED coverage statement is admitted here, before any
    // route is chosen, so no route can silently discard it.
    let declared_coverage = read_declared_coverage(&game_id, &index)?;

    // FAIL-CLOSED by default. Only an explicit portable certified pre-folder
    // can turn a verified compact-only container into immutable canonical
    // snapshots; replay_core never sees H4CR records or extractor fold types.
    if index.snapshots.is_empty() {
        // Every route below this point derives its hours from a decoder or
        // cache, not from `index.snapshots`, and none of them can carry an
        // index-declared gap. Refuse rather than drop the recorder's own
        // statement that hours are missing.
        if !declared_coverage.gaps.is_empty() {
            return Err(SourceError::Unsupported(format!(
                "recording {game_id:?} declares {} index coverage gap(s) but has no snapshot index \
                 entries; this build cannot carry declared coverage across the \
                 compact-stream/replay-cache routes",
                declared_coverage.gaps.len()
            )));
        }
        if packaged_replay_cache_roles_present(&store) {
            return open_packaged_replay_cache(
                store,
                game_id,
                index.schema_version.clone(),
                metadata,
                // A caller that supplied a spool base gets the non-resident
                // cache reader. The base is the caller's, and this route never
                // invents one.
                spool_decoder.map(|(base, _)| base),
            );
        }
        if let Some((spool_base, spool_decoder)) = spool_decoder {
            return open_compact_stream_with_spooled_decoder(
                store,
                game_id,
                index.schema_version.clone(),
                metadata,
                spool_base,
                spool_decoder,
            );
        }
        if let Some(decoder) = decoder {
            return open_compact_stream_with_decoder(
                store,
                game_id,
                index.schema_version.clone(),
                metadata,
                decoder,
            );
        }
        return Err(empty_index_error(
            &store,
            metadata.timeline.as_ref(),
            &game_id,
        ));
    }
    validate_index_schema_version(index.schema_version.as_ref())?;

    let mut files_by_hour: BTreeMap<i64, String> = BTreeMap::new();
    let mut dates_by_hour: BTreeMap<i64, Option<String>> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();
    for entry in &index.snapshots {
        match entry.hour_index {
            None => warnings.push(format!(
                "index entry {:?} has no hour_index; skipped (never guessed)",
                entry.file
            )),
            Some(hour) => {
                // REFUSE, never resolve. The index is the canonical authority
                // for an hour (REPLAY_SCHEMA.md §0.3); two rows claiming one
                // hour means the recording states two different truths about
                // it, and "keep the first" silently picks one — including its
                // date and its file — with no evidence that the first is the
                // right one. There is no rule that makes that choice sound.
                if let Some(existing) = files_by_hour.get(&hour) {
                    return Err(SourceError::Open(format!(
                        "recording {game_id:?} index names hour_index {hour} twice \
                         ({existing:?} and {:?}); the index is the canonical authority for an \
                         hour and this build refuses to choose between two claims",
                        entry.file
                    )));
                }
                files_by_hour.insert(hour, entry.file.clone());
                dates_by_hour.insert(hour, entry.date.clone());
            }
        }
    }
    // A STITCHED index is produced by this project's own stitcher and must
    // carry a date on every visible snapshot row. Without it `date_at` answers
    // `None` for the whole timeline and jump-to-date degrades into "seek to the
    // first hour" while still reporting success — an unanswerable navigation
    // query returning a plausible answer. Plain (v1) indexes keep the
    // historical tolerance: their dates are optional by contract.
    if index.is_stitched() {
        if let Some((hour, _)) = dates_by_hour
            .iter()
            .find(|(_, date)| date.as_deref().is_none_or(|date| date.trim().is_empty()))
        {
            return Err(SourceError::Open(format!(
                "stitched recording {game_id:?} visible snapshot hour {hour} carries no index \
                 date; a stitched index must date every visible hour (date navigation reads the \
                 index, never a frame body)"
            )));
        }
    }
    // A declared gap and a playable snapshot cannot both own an hour: the
    // stitched index "never lists a superseded snapshot and never hides a main
    // gap" (STITCHING_DESIGN.md §7.1).
    for gap in &declared_coverage.gaps {
        if let Some((hour, file)) = files_by_hour
            .range(gap.start_hour..=gap.end_hour)
            .next()
            .map(|(hour, file)| (*hour, file.clone()))
        {
            return Err(SourceError::Open(format!(
                "recording {game_id:?} declares hours {}..={} absent ({}: {}) yet its index also \
                 names a playable snapshot {file:?} at hour {hour}",
                gap.start_hour, gap.end_hour, gap.kind, gap.reason
            )));
        }
    }
    if files_by_hour.is_empty() {
        return Err(SourceError::Unsupported(format!(
            "recording {game_id:?} has {} snapshot index entries but none carries an \
             hour_index; nothing is playable (hour_index is the canonical key and is \
             never guessed)",
            index.snapshots.len()
        )));
    }

    let hours: Vec<i64> = files_by_hour.keys().copied().collect();
    let static_artifacts = metadata.admit_static_artifacts(&store, &game_id)?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        &hours,
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    Ok(Box::new(StoreSource {
        store,
        game_id,
        hours,
        files_by_hour,
        dates_by_hour,
        declared_coverage,
        warnings,
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
    }))
}

fn open_compact_stream_with_decoder(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    mut metadata: MetadataRoute,
    decoder: &dyn snapshot::CertifiedPrefoldDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let identity_mode = prefold_stream_identity_mode(&metadata);
    let container = store.container().ok_or_else(|| {
        SourceError::Unsupported(format!(
            "recording {game_id:?} is a compact-stream-only directory store; the portable decoder seam accepts authenticated .battleplan container role entries only"
        ))
    })?;
    let entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
    let entry = match entries.as_slice() {
        [entry] => entry,
        // This route folds the raw H4CR itself, so it needs the stream's
        // CONTENT and cannot serve a cache-only recording. A recording that
        // omitted the stream is normally served by the packaged-cache route
        // instead (its H4IX/H4CP roles are selected first), so reaching here
        // means the cache roles are missing too — say which of the two is
        // actually wrong rather than reporting a bare absent entry.
        [] => {
            return Err(SourceError::Unsupported(
                match declared_raw_stream_packaging(metadata.recorder_manifest.as_ref()) {
                    Ok(snapshot::RawStreamPackaging::Omitted) => format!(
                        "recording {game_id:?} {}",
                        snapshot::raw_stream_unavailable_message(
                            "be folded through the raw-stream decoder seam"
                        ),
                    ),
                    _ => format!(
                        "recording {game_id:?} has an empty snapshot index but no manifest entry with role {:?}",
                        snapshot::COMPACT_STREAM_ROLE
                    ),
                },
            ))
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recording {game_id:?} has {} manifest entries with role {:?}; expected exactly one",
                entries.len(),
                snapshot::COMPACT_STREAM_ROLE
            )))
        }
    };
    validate_index_schema_version(declared_schema_version.as_ref())?;
    validate_compact_stream_entry_identity(&entry.path, &game_id, metadata.timeline.as_ref())?;
    validate_recorder_compact_stream_join(
        &entry.path,
        entry.bytes,
        &entry.sha256,
        &game_id,
        metadata.recorder_manifest.as_ref(),
    )?;
    // Same shipped-playback ceiling the spooled sibling applies, admitted from
    // the MANIFEST length BEFORE any bytes are materialized. This route reads
    // the whole entry into memory, so skipping the admission here made the
    // in-memory path the more permissive of the two.
    admit_product_compact_stream_bytes(entry.bytes)?;
    let raw_h4cr = container.read_entry_bytes(&entry.path).map_err(|error| {
        let message = format!(
            "read verified compact stream role entry {:?}: {error}",
            entry.path
            );
        source_error_from_load(&snapshot::LoadError::Container(error), message)
    })?;
    let request = snapshot::CertifiedPrefoldRequest::try_new_with_identity_mode(
        game_id.clone(),
        raw_h4cr,
        identity_mode,
    )
    .map_err(|error| {
            SourceError::Open(format!(
                "compact stream role entry {:?} violates the portable pre-fold request: {error}",
                entry.path
            ))
        })?;
    let certified = snapshot::decode_certified_prefolded(decoder, request).map_err(|error| {
        SourceError::Open(format!(
            "portable decoder {:?} rejected compact stream role entry {:?}: {error}",
            decoder.decoder_id(),
            entry.path
        ))
    })?;
    validate_index_schema_version(Some(certified.index().schema_version()))?;
    if let Some(declared) = declared_schema_version.as_ref() {
        if !same_supported_schema_version(declared, certified.index().schema_version()) {
            return Err(SourceError::Open(format!(
                "compact-only index schema_version {declared} does not match decoder {:?} output schema_version {}",
                decoder.decoder_id(),
                certified.index().schema_version()
            )));
        }
    }
    validate_prefolded_container_identity(container, certified.replay())?;

    let static_artifacts = metadata.admit_static_artifacts(&store, &game_id)?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        certified.replay().index().hours(),
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    let prefold_receipt = Some(certified.receipt().clone());
    Ok(Box::new(PrefoldedSource {
        replay: certified.into_replay(),
        warnings: Vec::new(),
        store: Some(store),
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        prefold_receipt,
    }))
}

fn packaged_replay_cache_roles_present(store: &Store) -> bool {
    store.container().is_some_and(|container| {
        !container
            .entries_by_role(snapshot::HOUR_INDEX_ROLE)
            .is_empty()
            || !container
                .entries_by_role(snapshot::CHECKPOINT_CACHE_ROLE)
                .is_empty()
    })
}

/// Canonical lowercase hex SHA-256, the spelling every recorder receipt uses.
fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// What the recorder says this recording is composed of.
///
/// Read from the authenticated recorder manifest, strictly: a present value
/// must be one of the assigned spellings. A recording with no composition field
/// predates the field, and every such recording packaged the stream, so absence
/// resolves to `Packaged` — never to "whatever is in the container".
fn declared_raw_stream_packaging(
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<snapshot::RawStreamPackaging, SourceError> {
    let Some(recorder_manifest) = recorder_manifest else {
        // No recorder authority at all is the pre-existing legacy shape; the
        // stream-entry rules below are unchanged for it.
        return Ok(snapshot::RawStreamPackaging::Packaged);
    };
    snapshot::read_raw_stream_packaging_with_legacy_default(recorder_manifest).map_err(|error| {
        SourceError::Open(format!("recorder manifest states no usable composition: {error}"))
    })
}

/// Refuse a cache-only recording whose own documents still name a packaged
/// stream path.
fn validate_no_stream_path_is_claimed(
    recorder_manifest: Option<&Map<String, Value>>,
    timeline: Option<&Value>,
) -> Result<(), SourceError> {
    let claimed = |label: &str| {
        SourceError::Open(format!(
            "recording declares the {:?} composition but its {label} still names a packaged \
             stream path",
            snapshot::RawStreamPackaging::Omitted.as_str(),
        ))
    };
    // Present-but-malformed is not absent. `and_then(as_object)` would let
    // `"replay_entries": "garbage"` skip this check entirely, so a non-object
    // is rejected rather than waved through.
    if let Some(value) = recorder_manifest.and_then(|manifest| manifest.get("replay_entries")) {
        let entries = value.as_object().ok_or_else(|| {
            SourceError::Open("recorder replay_entries authority is not an object".to_string())
        })?;
        if entries.contains_key("stream") {
            return Err(claimed("recorder manifest replay_entries"));
        }
    }
    if let Some(timeline) = timeline {
        let timeline = timeline.as_object().ok_or_else(|| {
            SourceError::Open("recorder-owned timeline is not an object".to_string())
        })?;
        if timeline.contains_key("stream") {
            return Err(claimed("timeline"));
        }
    }
    Ok(())
}

/// The source length and SHA-256 the certified cache is bound to, taken from
/// the recorder manifest's capture receipt.
///
/// This is the same CLAIMED length and digest the packaged stream entry would
/// have carried — the recorder writes `stream_storage` from the very capture
/// receipt it also packs the stream against. What it is not is an independent
/// demonstration: with the stream absent there are no source bytes to re-hash,
/// so this identity is attested by the authenticated recorder manifest rather
/// than proven from the source itself.
fn recorder_manifest_source_identity(
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<(u64, String), SourceError> {
    let missing = || {
        SourceError::Open(format!(
            "recording declares the {:?} composition, so its source identity must come from the \
             recorder manifest, but stream_storage.decoded_bytes/decoded_sha256 is missing or \
             malformed",
            snapshot::RawStreamPackaging::Omitted.as_str(),
        ))
    };
    let storage = recorder_manifest
        .ok_or_else(missing)?
        .get("stream_storage")
        .and_then(Value::as_object)
        .ok_or_else(missing)?;
    let bytes = storage
        .get("decoded_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(missing)?;
    let sha256 = storage
        .get("decoded_sha256")
        .and_then(Value::as_str)
        .filter(|value| is_lower_sha256(value))
        .ok_or_else(missing)?
        .to_string();
    Ok((bytes, sha256))
}

fn open_packaged_replay_cache(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    mut metadata: MetadataRoute,
    cache_spool_base: Option<&Path>,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    validate_index_schema_version(declared_schema_version.as_ref())?;
    let identity_mode = prefold_stream_identity_mode(&metadata);
    let mut cache_spool: Option<PrefoldSpoolOwner> = None;
    let (cache, prefold_receipt, hours) = {
        let container = store.container().ok_or_else(|| {
            SourceError::Unsupported(
                "packaged replay_decoder artifacts require a portable container authority"
                    .to_string(),
            )
        })?;
        let select = |role: &str| -> Result<snapshot::container::ContainerEntry, SourceError> {
            match container.entries_by_role(role).as_slice() {
                [entry] => Ok(entry.clone()),
                [] => Err(SourceError::Open(format!(
                    "packaged replay_decoder container omits required role {role:?}"
                ))),
                entries => Err(SourceError::Open(format!(
                    "packaged replay_decoder container has {} entries for unique role {role:?}",
                    entries.len()
                ))),
            }
        };
        let index_entry = select(snapshot::HOUR_INDEX_ROLE)?;
        let cache_entry = select(snapshot::CHECKPOINT_CACHE_ROLE)?;
        let expected_prefix = format!("payload/{game_id}/");
        let expected_index_path = format!("{expected_prefix}{}", snapshot::HOUR_INDEX_FILE);
        let expected_cache_path =
            format!("{expected_prefix}{}", snapshot::CHECKPOINT_CACHE_FILE);
        if index_entry.path != expected_index_path || cache_entry.path != expected_cache_path {
            return Err(SourceError::Open(
                "packaged replay_decoder role paths disagree with the selected game namespace"
                    .to_string(),
            ));
        }
        // What this recording says it is made of, read strictly from the
        // authenticated recorder manifest. A recording that states nothing is
        // treated as pre-composition, and those all carried the stream.
        let packaging = declared_raw_stream_packaging(metadata.recorder_manifest.as_ref())?;
        let stream_entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
        // The cache is bound to its source by LENGTH and SHA-256, not by the
        // source's bytes: hours are reconstructed from H4IX/H4CP, and the H4CR
        // entry is never parsed on this route. So a cache-only recording is
        // admitted on that identity, taken from the recorder manifest — itself
        // a container-authenticated entry — and `CertifiedReplayCache::open`
        // cross-checks it before a single hour is served: the length against
        // the H4IX header, and the SHA-256 against BOTH the H4IX header and the
        // H4CP header. (H4CP carries the source digest but not the source
        // length, so the length rests on H4IX alone.)
        //
        // What is NOT done for a cache-only recording, and cannot be, is
        // re-hashing the source's own bytes: they are not in the container.
        // The packaged branch below does that via `authenticate_entry`. So the
        // cache's internal integrity is identical under both compositions,
        // while the claimed H4CR source identity is attested by the recorder
        // manifest rather than independently demonstrated from source bytes.
        // That is the price of the composition, and it is stated here rather
        // than left for a reader to discover.
        //
        // The composition is authority in BOTH directions: a declared stream
        // that is missing, and an undeclared stream that is present, are each a
        // hard error. Neither can be reached by an entry silently appearing or
        // disappearing.
        let source_identity = match (packaging, stream_entries.as_slice()) {
            (snapshot::RawStreamPackaging::Packaged, [source_entry]) => {
                validate_compact_stream_entry_identity(
                    &source_entry.path,
                    &game_id,
                    metadata.timeline.as_ref(),
                )?;
                validate_recorder_compact_stream_join(
                    &source_entry.path,
                    source_entry.bytes,
                    &source_entry.sha256,
                    &game_id,
                    metadata.recorder_manifest.as_ref(),
                )?;
                let source_receipt = container
                    .authenticate_entry(&source_entry.path)
                    .map_err(|error| {
                        SourceError::Open(format!(
                            "authenticate canonical H4CR role before replay-cache admission: {error}"
                        ))
                    })?;
                if source_receipt.uncompressed_len() != source_entry.bytes
                    || source_receipt.sha256() != source_entry.sha256
                {
                    return Err(SourceError::Open(
                        "canonical H4CR authentication receipt disagrees with its container role"
                            .to_string(),
                    ));
                }
                (source_entry.bytes, source_entry.sha256.clone())
            }
            (snapshot::RawStreamPackaging::Packaged, []) => {
                return Err(SourceError::Open(format!(
                    "packaged replay_decoder container omits required role {:?}, but the recorder \
                     manifest declares the {:?} composition",
                    snapshot::COMPACT_STREAM_ROLE,
                    snapshot::RawStreamPackaging::Packaged.as_str(),
                )))
            }
            (snapshot::RawStreamPackaging::Omitted, []) => {
                // A cache-only recording must not NAME a stream entry either.
                // The composition and the paths are two ways of saying the same
                // thing, and a document that still points at
                // `collector_raw/collector_stream.bin` is describing a
                // container that does not exist.
                validate_no_stream_path_is_claimed(
                    metadata.recorder_manifest.as_ref(),
                    metadata.timeline.as_ref(),
                )?;
                recorder_manifest_source_identity(metadata.recorder_manifest.as_ref())?
            }
            (snapshot::RawStreamPackaging::Omitted, _) => {
                return Err(SourceError::Open(format!(
                    "recorder manifest declares the {:?} composition, but the container carries \
                     {} entries with role {:?}",
                    snapshot::RawStreamPackaging::Omitted.as_str(),
                    stream_entries.len(),
                    snapshot::COMPACT_STREAM_ROLE,
                )))
            }
            (_, entries) => {
                return Err(SourceError::Open(format!(
                    "packaged replay_decoder container has {} entries for unique role {:?}",
                    entries.len(),
                    snapshot::COMPACT_STREAM_ROLE,
                )))
            }
        };
        let (source_len, source_sha256) = source_identity;
        let index_bytes = container
            .read_entry_bytes(&index_entry.path)
            .map_err(|error| {
                SourceError::Open(format!(
                    "read certified replay_decoder hour index: {error}"
                ))
            })?;
        // The checkpoint cache is the one packaged entry whose size tracks
        // campaign length rather than schema: capture t32 emitted 2,209,767,138
        // bytes for 656 game hours, so a full campaign's cache is tens of
        // gigabytes and reading it into a `Vec` is not a thing a supported
        // machine can do. When the caller owns a scratch base, the entry is
        // authenticated into a private spool and every seek reads one bounded
        // record from it. Without a base the old resident read stands, and the
        // container reader's 2 GiB materialization cap refuses an oversized
        // cache BY NAME rather than truncating it.
        // THE CONTAINER'S ACCOUNT OF THIS ENTRY IS NOT THE LAST WORD ON IT
        // (task #192, HA-3).
        //
        // Both routes used to authenticate the entry against the container
        // manifest and refuse the WHOLE recording the moment that digest
        // disagreed — the spool route inside
        // `spool_entry_to_file_transactionally_bounded`, the resident route
        // inside `read_entry_bytes`. A cache entry that rotted after packing
        // therefore cost every hour of the recording, and the per-record
        // confinement lanes GZ and HA built was never reached, because it lives
        // one call further in, in `CertifiedReplayCache`'s admission.
        //
        // The order is inverted here for the reason
        // `compact_replay_verification_gates`' link 6 already states about the
        // same bytes: the sealed H4IX carries a digest per cache record, and
        // that is a STRICTLY STRONGER authority than the container manifest's
        // one digest over the whole entry. A record whose bytes hash to the
        // digest the SEAL published is that record, whatever the container's
        // own account of the entry says. So the entry is streamed into a
        // caller-owned sink that is never published on this route, the copy's
        // outcome is KEPT AS A FACT instead of ending the open, and
        // `CertifiedReplayCache` — the one door that can attribute damage to
        // hours — is what judges the bytes.
        //
        // No unverified byte is ever served by this: every hour that
        // reconstructs has had its record checked against the sealed
        // per-record digest, and every hour that has not is refused by name.
        // The two accounts disagreeing in the OTHER directions is a
        // whole-recording refusal, in the four-outcome match below.
        let (cache, copy_outcome) = match cache_spool_base {
            Some(base) => {
                let owner = PrefoldSpoolOwner::create(base)?;
                let spooled = owner.root().join("checkpoints.h4cp");
                let copy_outcome = {
                    let mut sink = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&spooled)
                        .map_err(|error| {
                            SourceError::Open(format!(
                                "create private checkpoint cache spool {}: {error}",
                                spooled.display()
                            ))
                        })?;
                    let outcome = container.copy_entry_to_unpublished_writer_bounded(
                        &cache_entry.path,
                        &mut sink,
                        snapshot::zip::STREAMING_MAX_CONTAINER_ENTRY_BYTES,
                    );
                    sink.sync_all().map_err(|error| {
                        SourceError::Open(format!(
                            "sync private checkpoint cache spool {}: {error}",
                            spooled.display()
                        ))
                    })?;
                    outcome
                };
                // When the container DID vouch for the entry, its receipt must
                // still be the entry the role index names. This check is not
                // weakened by the reordering; it is only skipped when there is
                // no receipt to check.
                if let Ok(receipt) = &copy_outcome {
                    if receipt.path() != cache_entry.path.as_str()
                        || receipt.uncompressed_len() != cache_entry.bytes
                        || receipt.sha256() != cache_entry.sha256
                    {
                        return Err(SourceError::Open(
                            "checkpoint cache spool receipt does not match its container role entry"
                                .to_string(),
                        ));
                    }
                }
                let file = open_private_prefold_spool_file(&spooled)?;
                let cache = snapshot::CertifiedReplayCache::open_file(
                    &index_bytes,
                    file,
                    source_len,
                    &source_sha256,
                )
                .map_err(|error| {
                    SourceError::Open(admission_refusal(&error, &copy_outcome))
                })?;
                cache_spool = Some(owner);
                (cache, copy_outcome)
            }
            None => {
                let mut cache_bytes = Vec::new();
                let copy_outcome = container.copy_entry_to_unpublished_writer_bounded(
                    &cache_entry.path,
                    &mut cache_bytes,
                    snapshot::zip::STREAMING_MAX_CONTAINER_ENTRY_BYTES,
                );
                let cache = snapshot::CertifiedReplayCache::open(
                    &index_bytes,
                    cache_bytes,
                    source_len,
                    &source_sha256,
                )
                .map_err(|error| {
                    SourceError::Open(admission_refusal(&error, &copy_outcome))
                })?;
                (cache, copy_outcome)
            }
        };
        // THE FOUR OUTCOMES of one packaged cache judged by two accounts, each
        // named. Two are agreements; two are the container's own account of the
        // entry and the seal's account of the same bytes contradicting each
        // other, which is attributable to no hour and refuses the recording.
        //
        // This is the same match `compact_replay_verification_gates`' link 6
        // makes over the same entry, and it is made here as well because the
        // gate certifies a recording once while this door opens one every time
        // a reader is constructed.
        match (&copy_outcome, cache.confined_hours().is_empty()) {
            // The container verified the entry and every record is the sealed
            // one.
            (Ok(_), true) => {}
            // The container refused the entry and the sealed index says exactly
            // which records moved. This is HA-3's case, and confining it is the
            // point of the reordering.
            (Err(_), false) => {}
            (Ok(receipt), false) => {
                return Err(SourceError::Open(format!(
                    "the container verified {} against its own manifest digest {}, yet the \
                     sealed H4IX refuses {} of its hours by per-record digest; the two accounts \
                     of the same bytes disagree, so neither is trusted",
                    cache_entry.path,
                    receipt.sha256(),
                    cache.confined_hours().len(),
                )))
            }
            (Err(copy_error), true) => {
                return Err(SourceError::Open(format!(
                    "the container refused {} ({copy_error}), but every record in it matches the \
                     per-record digest the sealed H4IX publishes; the damage is in the \
                     container's own account of the entry, which serves no single game hour",
                    cache_entry.path,
                )))
            }
        }
        if cache.index().game_id() != game_id
            || cache.index().schema_version() != "1"
            || cache.index().identity_mode()
                != match identity_mode {
                    snapshot::PrefoldStreamIdentityMode::Legacy => {
                        snapshot::CacheIdentityMode::Legacy
                    }
                    snapshot::PrefoldStreamIdentityMode::IdentityV1 => {
                        snapshot::CacheIdentityMode::IdentityV1
                    }
                }
        {
            return Err(SourceError::Open(
                "packaged replay_decoder identity disagrees with container/recorder authority"
                    .to_string(),
            ));
        }
        validate_packaged_replay_cache_index_descriptor(
            &store,
            &game_id,
            cache.index(),
            &index_entry,
            &cache_entry,
            packaging,
        )?;
        let hours = cache
            .index()
            .hours()
            .iter()
            .map(snapshot::ReplayHourEntry::hour_index)
            .collect::<Vec<_>>();
        let receipt = snapshot::PrefoldSourceReceipt::try_new(
            "extractor.replay_decoder.binary_seek.v1",
            source_len,
            source_sha256,
            identity_mode,
        )
        .map_err(|error| SourceError::Open(error.to_string()))?;
        (cache, receipt, hours)
    };

    let first_json = cache
        .reconstruct_json(hours[0])
        .map_err(|error| SourceError::Open(format!("reconstruct first replay hour: {error}")))?
        .0;
    let first: snapshot::Snapshot = snapshot::parse_json_strict(&first_json)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            SourceError::Open(format!(
                "first binary replay reconstruction is not canonical snapshot JSON: {error}"
            ))
        })?;
    validate_snapshot_container_identity(
        store
            .container()
            .expect("packaged replay cache requires a container"),
        &first,
    )?;
    let static_artifacts = metadata.admit_static_artifacts(&store, &game_id)?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        &hours,
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    Ok(Box::new(IndexedReplaySource {
        cache,
        // Held for exactly as long as the cache that reads from it; dropping the
        // source removes the private spool directory.
        cache_spool,
        hours,
        warnings: Vec::new(),
        store,
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        prefold_receipt,
    }))
}

fn validate_packaged_replay_cache_index_descriptor(
    store: &Store,
    game_id: &str,
    replay_index: &snapshot::ReplayHourIndex,
    index_entry: &snapshot::container::ContainerEntry,
    cache_entry: &snapshot::container::ContainerEntry,
    declared_packaging: snapshot::RawStreamPackaging,
) -> Result<(), SourceError> {
    let bytes = store
        .read_game_file_bytes(game_id, "index.json")
        .map_err(|error| {
            let message = format!("read replay index descriptor: {error}");
            source_error_from_load(&error, message)
        })?
        .ok_or_else(|| SourceError::Open("replay index descriptor disappeared".to_string()))?;
    let value = snapshot::parse_json_strict(&bytes)
        .map_err(|error| SourceError::Open(format!("strict replay index descriptor: {error}")))?;
    // index.json is the third document that states the composition. The
    // timeline and the recorder manifest are already cross-checked against each
    // other; without this, index.json would be the one copy a rewrite could
    // change unnoticed.
    let index_packaging = value
        .as_object()
        .ok_or_else(|| SourceError::Open("replay index descriptor is not an object".to_string()))
        .and_then(|object| {
            snapshot::read_raw_stream_packaging_with_legacy_default(object).map_err(|error| {
                SourceError::Open(format!("replay index descriptor composition: {error}"))
            })
        })?;
    if index_packaging != declared_packaging {
        return Err(SourceError::Open(format!(
            "replay index descriptor declares the {:?} composition but the recorder authority              declares {:?}",
            index_packaging.as_str(),
            declared_packaging.as_str(),
        )));
    }
    let no_legacy_snapshots = value
        .get("snapshots")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
        && value
            .get("production_json_snapshots_omitted")
            .and_then(Value::as_bool)
            == Some(true)
        && value
            .get("canonical_world_source")
            .and_then(Value::as_str)
            == Some(snapshot::COMPACT_STREAM_ROLE)
        && store
            .container()
            .is_some_and(|container| container.entries_by_role("snapshot").is_empty());
    let decoder = value
        .get("replay_decoder")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(
                "empty snapshot index has replay-cache roles but no replay_decoder descriptor"
                    .to_string(),
            )
        })?;
    let hour = decoder
        .get("hour_index")
        .and_then(Value::as_object)
        .ok_or_else(|| SourceError::Open("replay_decoder hour_index descriptor missing".into()))?;
    let checkpoints = decoder
        .get("checkpoints")
        .and_then(Value::as_object)
        .ok_or_else(|| SourceError::Open("replay_decoder checkpoints descriptor missing".into()))?;
    let exact = no_legacy_snapshots
        && decoder.get("schema").and_then(Value::as_str)
        == Some("hoi4.replay_decoder.binary_seek.v1")
        && decoder
            .get("exact_reconstruction_parity")
            .and_then(Value::as_bool)
            == Some(true)
        && decoder
            .get("bounded_seek_certified")
            .and_then(Value::as_bool)
            == Some(true)
        && hour.get("role").and_then(Value::as_str) == Some(snapshot::HOUR_INDEX_ROLE)
        && hour.get("path").and_then(Value::as_str) == Some(snapshot::HOUR_INDEX_FILE)
        && hour.get("bytes").and_then(Value::as_u64) == Some(index_entry.bytes)
        && hour.get("sha256").and_then(Value::as_str) == Some(index_entry.sha256.as_str())
        && hour.get("hours").and_then(Value::as_u64)
            == Some(replay_index.hours().len() as u64)
        && hour.get("source_bytes").and_then(Value::as_u64)
            == Some(replay_index.source_len())
        && hour.get("source_sha256").and_then(Value::as_str)
            == Some(replay_index.source_sha256())
        && checkpoints.get("role").and_then(Value::as_str)
            == Some(snapshot::CHECKPOINT_CACHE_ROLE)
        && checkpoints.get("path").and_then(Value::as_str)
            == Some(snapshot::CHECKPOINT_CACHE_FILE)
        && checkpoints.get("bytes").and_then(Value::as_u64) == Some(cache_entry.bytes)
        && checkpoints.get("sha256").and_then(Value::as_str)
            == Some(cache_entry.sha256.as_str())
        && checkpoints
            .get("checkpoint_interval_hours")
            .and_then(Value::as_u64)
            == Some(replay_index.checkpoint_interval_hours() as u64)
        && checkpoints
            .get("max_seek_deltas")
            .and_then(Value::as_u64)
            .is_some_and(|value| value < replay_index.checkpoint_interval_hours() as u64);
    if !exact {
        return Err(SourceError::Open(
            "replay_decoder index descriptor does not exactly join its certified roles or still carries production JSON snapshots"
                .to_string(),
        ));
    }
    Ok(())
}

fn open_compact_stream_with_spooled_decoder(
    store: Store,
    game_id: String,
    declared_schema_version: Option<Value>,
    mut metadata: MetadataRoute,
    spool_base: &Path,
    decoder: &dyn snapshot::CertifiedPrefoldSpoolDecoder,
) -> Result<Box<dyn ReplaySource>, SourceError> {
    let identity_mode = prefold_stream_identity_mode(&metadata);
    let owner = PrefoldSpoolOwner::create(spool_base)?;
    let raw_path = owner.root().join("source.h4cr");
    let snapshot_dir = owner.snapshot_dir().to_path_buf();

    let source_receipt = {
        let container = store.container().ok_or_else(|| {
            SourceError::Unsupported(format!(
                "recording {game_id:?} is a compact-stream-only directory store; the bounded portable decoder seam accepts authenticated .battleplan role entries only"
            ))
        })?;
        let entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
        let entry = match entries.as_slice() {
            [entry] => entry,
            // Same as the unspooled seam: this performs a real H4CR fold, so a
            // cache-only recording is refused by name rather than by absence.
            [] => {
                return Err(SourceError::Unsupported(
                    match declared_raw_stream_packaging(metadata.recorder_manifest.as_ref()) {
                        Ok(snapshot::RawStreamPackaging::Omitted) => format!(
                            "recording {game_id:?} {}",
                            snapshot::raw_stream_unavailable_message(
                                "be folded through the bounded raw-stream decoder seam"
                            ),
                        ),
                        _ => format!(
                            "recording {game_id:?} has an empty snapshot index but no manifest entry with role {:?}",
                            snapshot::COMPACT_STREAM_ROLE
                        ),
                    },
                ))
            }
            _ => {
                return Err(SourceError::Open(format!(
                    "recording {game_id:?} has {} manifest entries with role {:?}; expected exactly one",
                    entries.len(),
                    snapshot::COMPACT_STREAM_ROLE
                )))
            }
        };
        validate_index_schema_version(declared_schema_version.as_ref())?;
        validate_compact_stream_entry_identity(&entry.path, &game_id, metadata.timeline.as_ref())?;
        validate_recorder_compact_stream_join(
            &entry.path,
            entry.bytes,
            &entry.sha256,
            &game_id,
            metadata.recorder_manifest.as_ref(),
        )?;
        admit_product_compact_stream_bytes(entry.bytes)?;
        let receipt = container
            .spool_entry_to_file_transactionally_bounded(
                &entry.path,
                &raw_path,
                PRODUCT_COMPACT_STREAM_MAX_BYTES,
            )
            .map_err(|error| {
                let message = format!(
                    "authenticate compact stream role entry {:?} into private spool: {error}",
                    entry.path
                );
                source_error_from_container(&error, message)
            })?;
        if receipt.path() != entry.path.as_str() {
            return Err(SourceError::Open(
                "container spool receipt path does not match selected compact role entry"
                    .to_string(),
            ));
        }
        receipt
    };

    let request = snapshot::CertifiedPrefoldSpoolRequest::try_new_with_identity_mode(
        game_id.clone(),
        raw_path,
        snapshot_dir.clone(),
        source_receipt.uncompressed_len(),
        source_receipt.sha256(),
        identity_mode,
    )
    .map_err(|error| {
        SourceError::Open(format!(
            "authenticated compact stream violates bounded pre-fold request: {error}"
        ))
    })?;
    let certified =
        snapshot::decode_certified_prefolded_spool(decoder, request).map_err(|error| {
            SourceError::Open(format!(
                "bounded portable decoder {:?} rejected authenticated compact stream: {error}",
                decoder.decoder_id()
            ))
        })?;
    validate_index_schema_version(Some(certified.spool().schema_version()))?;
    if let Some(declared) = declared_schema_version.as_ref() {
        if !same_supported_schema_version(declared, certified.spool().schema_version()) {
            return Err(SourceError::Open(format!(
                "compact-only index schema_version {declared} does not match bounded decoder {:?} output schema_version {}",
                decoder.decoder_id(),
                certified.spool().schema_version()
            )));
        }
    }
    if certified.snapshot_dir() != snapshot_dir {
        return Err(SourceError::Open(
            "bounded decoder result changed the caller-owned snapshot spool path".to_string(),
        ));
    }
    let (spool, certified_snapshot_dir, prefold_receipt) = certified.into_parts();
    if prefold_receipt.source_len() != source_receipt.uncompressed_len()
        || prefold_receipt.source_sha256() != source_receipt.sha256()
    {
        return Err(SourceError::Open(
            "bounded decoder receipt is not bound to the authenticated container entry".to_string(),
        ));
    }
    owner.remove_authenticated_source()?;

    let first_entry = spool.entries().first().expect("portable spool is nonempty");
    let first = read_spooled_hour(&owner, &certified_snapshot_dir, &spool, first_entry)?;
    validate_snapshot_container_identity(
        store
            .container()
            .expect("spooled compact route requires container"),
        &first,
    )?;

    let spool_hours = spool
        .entries()
        .iter()
        .map(|entry| entry.hour_index())
        .collect::<Vec<_>>();
    let static_artifacts = metadata.admit_static_artifacts(&store, &game_id)?;
    let design_catalog = read_portable_design_catalog(
        &store,
        &game_id,
        &spool_hours,
        metadata.recorder_manifest.as_ref(),
    )?;
    let decision_catalog =
        read_packaged_decision_catalog(&store, &game_id, metadata.recorder_manifest.as_ref())?;
    Ok(Box::new(SpooledPrefoldedSource {
        spool,
        snapshot_dir: certified_snapshot_dir,
        owner,
        warnings: Vec::new(),
        store,
        recorder_binding: metadata.binding,
        design_catalog,
        decision_catalog,
        static_artifacts,
        prefold_receipt,
    }))
}

fn prefold_stream_identity_mode(metadata: &MetadataRoute) -> snapshot::PrefoldStreamIdentityMode {
    prefold_stream_identity_mode_from_campaign(
        metadata
            .binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref()),
    )
}

fn prefold_stream_identity_mode_from_campaign(
    identity: Option<&RecordedCampaignIdentity>,
) -> snapshot::PrefoldStreamIdentityMode {
    match identity.map(|identity| identity.state.as_str()) {
        Some("known") => snapshot::PrefoldStreamIdentityMode::IdentityV1,
        _ => snapshot::PrefoldStreamIdentityMode::Legacy,
    }
}

fn validate_compact_stream_entry_identity(
    entry_path: &str,
    game_id: &str,
    timeline: Option<&Value>,
) -> Result<(), SourceError> {
    let game_prefix = format!("payload/{game_id}/");
    if !entry_path.starts_with(&game_prefix) {
        return Err(SourceError::Open(format!(
            "compact stream role entry {entry_path:?} is outside manifest game namespace {game_prefix:?}"
        )));
    }

    let Some(timeline) = timeline.and_then(Value::as_object) else {
        return Ok(());
    };
    let format = timeline.get("format").and_then(Value::as_str);
    let role = timeline.get("raw_stream_role").and_then(Value::as_str);
    let declares_compact =
        format == Some(COMPACT_STREAM_TIMELINE_FORMAT) || role == Some(COMPACT_STREAM_ROLE);
    if !declares_compact {
        return Ok(());
    }
    if format != Some(COMPACT_STREAM_TIMELINE_FORMAT) || role != Some(COMPACT_STREAM_ROLE) {
        return Err(SourceError::Open(format!(
            "compact timeline identity is incomplete: format={format:?}, raw_stream_role={role:?}"
        )));
    }
    let relative = timeline
        .get("stream")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            SourceError::Open("compact timeline declares no nonblank stream path".to_string())
        })?;
    if relative.starts_with('/')
        || relative.contains(char::from(92))
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || snapshot::zip::validate_path(relative).is_err()
    {
        return Err(SourceError::Open(format!(
            "compact timeline stream path is not a valid game-relative forward-slash path: {relative:?}"
        )));
    }
    let expected_path = format!("{game_prefix}{relative}");
    if entry_path != expected_path {
        return Err(SourceError::Open(format!(
            "compact stream role entry {entry_path:?} does not byte-match timeline stream {expected_path:?}"
        )));
    }
    Ok(())
}

/// Prove container identity for EVERY prefolded hour, not just the first.
///
/// The whole prefolded set is already materialized on this route, so checking
/// only entry 0 left every later hour's provenance unjoined to the container
/// manifest for no saving at all.
fn validate_prefolded_container_identity(
    container: &snapshot::Container,
    replay: &snapshot::PrefoldedReplay,
) -> Result<(), SourceError> {
    for entry in replay.entries() {
        validate_snapshot_container_identity(container, entry.snapshot())?;
    }
    Ok(())
}

fn validate_snapshot_container_identity(
    container: &snapshot::Container,
    first: &snapshot::Snapshot,
) -> Result<(), SourceError> {
    let manifest = container.manifest();
    let provenance = first.rest.get("provenance");
    let manifest_modset = manifest
        .get("mod_identity")
        .and_then(|value| value.get("modset_id"))
        .and_then(Value::as_str);
    let snapshot_modset = provenance
        .and_then(|value| value.get("mod"))
        .and_then(|value| value.get("modset_id"))
        .and_then(Value::as_str);
    if let (Some(manifest), Some(snapshot_modset_id)) = (manifest_modset, snapshot_modset) {
        if manifest != snapshot_modset_id {
            let error = snapshot::ContainerError::ModsetMismatch {
                manifest: manifest.to_string(),
                snapshot: snapshot_modset_id.to_string(),
            };
            return Err(SourceError::ModsetMismatch(format!(
                "open .battleplan: {error}"
            )));
        }
    }

    let manifest_sha = manifest
        .get("build")
        .and_then(|value| value.get("exe_sha256"))
        .and_then(Value::as_str);
    for (label, snapshot_sha) in [
        (
            "collector",
            provenance
                .and_then(|value| value.get("collector"))
                .and_then(|value| value.get("exe_sha256"))
                .and_then(Value::as_str),
        ),
        (
            "memory",
            provenance
                .and_then(|value| value.get("memory"))
                .and_then(|value| value.get("exe_sha256"))
                .and_then(Value::as_str),
        ),
    ] {
        if let (Some(manifest), Some(snapshot)) = (manifest_sha, snapshot_sha) {
            if manifest != snapshot {
                return Err(SourceError::Open(format!(
                    "open .battleplan: container rejected: manifest build.exe_sha256 {manifest:?} disagrees with prefolded snapshot provenance.{label}.exe_sha256 {snapshot:?}"
                )));
            }
        }
    }
    Ok(())
}

fn read_portable_design_catalog(
    store: &Store,
    game_id: &str,
    hours: &[i64],
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<Option<Arc<crate::design_catalog::PortableDesignCatalog>>, SourceError> {
    let recorder_row = recorder_manifest
        .and_then(|manifest| manifest.get("replay_packaging"))
        .and_then(|packaging| packaging.get("design_catalog_live"))
        .filter(|value| !value.is_null());

    // Legacy stores without a recorder binding keep the historical optional
    // path. A recorder-owned package must join recorder seal, selected index,
    // dedicated role, content-manifest epoch, and the verified payload bytes.
    if recorder_manifest.is_some() {
        let index_bytes = store
            .read_game_file_bytes(game_id, "index.json")
            .map_err(|error| {
                let message = format!(
                    "read authenticated index for A27 design-catalog join: {error}"
                );
                source_error_from_load(&error, message)
            })?
            .ok_or_else(|| {
                SourceError::Open(
                    "selected game index disappeared during A27 design-catalog join".into(),
                )
            })?;
        let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
            SourceError::Open(format!(
                "strict parse selected game index for A27 design-catalog join: {error}"
            ))
        })?;
        let index_row = index
            .get("design_catalog_live")
            .filter(|value| value.get("present").and_then(Value::as_bool) == Some(true));
        match (recorder_row, index_row) {
            (None, None) => {
                if store
                    .container()
                    .is_some_and(|container| {
                        !container.entries_by_role("design_catalog_live").is_empty()
                    })
                {
                    return Err(SourceError::Open(
                        "A27 design-catalog role exists without recorder/index authority".into(),
                    ));
                }
                return Ok(None);
            }
            (None, Some(_)) => {
                return Err(SourceError::Open(
                    "index records A27 design catalog present but recorder manifest omits it"
                        .into(),
                ))
            }
            (Some(_), None) => {
                return Err(SourceError::Open(
                    "recorder manifest records A27 design catalog but index omits it".into(),
                ))
            }
            (Some(recorder_row), Some(index_row)) => {
                let expected_path = crate::design_catalog::PORTABLE_ARTIFACT_PATH;
                let recorder_path = recorder_row.get("path").and_then(Value::as_str);
                let recorder_sha = recorder_row.get("sha256").and_then(Value::as_str);
                let recorder_len = recorder_row.get("bytes").and_then(Value::as_u64);
                let recorder_content_digest = recorder_row
                    .get("content_manifest_digest_sha256")
                    .and_then(Value::as_str);
                if recorder_path != Some(expected_path)
                    || recorder_sha.is_none_or(|value| !lower_sha256(value))
                    || recorder_len.is_none()
                    || recorder_content_digest.is_none_or(|value| !lower_sha256(value))
                {
                    return Err(SourceError::Open(
                        "recorder manifest A27 design-catalog identity is invalid".into(),
                    ));
                }
                let recorder_sha = recorder_sha.expect("validated A27 sha");
                let recorder_len = recorder_len.expect("validated A27 byte length");
                let recorder_content_digest =
                    recorder_content_digest.expect("validated A27 content digest");
                if index_row.get("role").and_then(Value::as_str)
                    != Some("design_catalog_live")
                    || index_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || index_row.get("sha256").and_then(Value::as_str) != Some(recorder_sha)
                    || index_row.get("bytes").and_then(Value::as_u64) != Some(recorder_len)
                    || index_row.get("source_sha256").and_then(Value::as_str)
                        != recorder_row.get("source_sha256").and_then(Value::as_str)
                {
                    return Err(SourceError::Open(
                        "index A27 design-catalog identity disagrees with recorder manifest"
                            .into(),
                    ));
                }

                let container = store.container().ok_or_else(|| {
                    SourceError::Open(
                        "A27 design catalog requires a portable container authority".into(),
                    )
                })?;
                let content_digest = container
                    .content_manifest()
                    .and_then(|manifest| manifest.manifest_digest_sha256.as_deref())
                    .ok_or_else(|| {
                        SourceError::Open(
                            "A27 design catalog has no finalized portable content-manifest epoch"
                                .into(),
                        )
                    })?;
                if content_digest != recorder_content_digest {
                    return Err(SourceError::Open(
                        "A27 design catalog is not bound to the selected content manifest".into(),
                    ));
                }
                let container_path = format!("payload/{game_id}/{expected_path}");
                let entries = container
                    .entries_by_role("design_catalog_live")
                    .into_iter()
                    .filter(|entry| entry.path == container_path)
                    .collect::<Vec<_>>();
                let entry = match entries.as_slice() {
                    [entry] => entry,
                    [] => {
                        return Err(SourceError::Open(
                            "A27 design-catalog index row has no matching dedicated role".into(),
                        ))
                    }
                    _ => {
                        return Err(SourceError::Open(
                            "A27 design catalog has duplicate dedicated role entries".into(),
                        ))
                    }
                };
                if entry.sha256 != recorder_sha || entry.bytes != recorder_len {
                    return Err(SourceError::Open(
                        "A27 design-catalog role entry disagrees with recorder/index hashes"
                            .into(),
                    ));
                }
            }
        }
    }

    // No unauthenticated catalog, ever. `Store::read_game_file_bytes`
    // authenticates a CONTAINER entry against its manifest digest, but for a
    // directory store it is an anchored file read with no hash or length
    // authority at all — the A27 catalog is a semantic authority the fold reads
    // for every frame, so admitting it from loose bytes lets an edited file on
    // disk redefine designs with nothing to detect it. A recorder-manifest row
    // supplies its own sha256/length (verified below), which is an independent
    // authority; without either, refuse.
    if store.container().is_none() && recorder_row.is_none() {
        let present = store
            .read_game_file_bytes(game_id, crate::design_catalog::PORTABLE_ARTIFACT_PATH)
            .map_err(|error| {
                source_error_from_load(
                    &error,
                    format!(
                        "probe A27 design catalog artifact {:?}: {error}",
                        crate::design_catalog::PORTABLE_ARTIFACT_PATH
                    ),
                )
            })?
            .is_some();
        if present {
            return Err(SourceError::Open(format!(
                "A27 design catalog {:?} exists in a directory store with no container entry \
                 digest and no recorder-manifest sha256/length; this build refuses to admit an \
                 unauthenticated semantic catalog",
                crate::design_catalog::PORTABLE_ARTIFACT_PATH
            )));
        }
        return Ok(None);
    }

    let bytes = store
        .read_game_file_bytes(game_id, crate::design_catalog::PORTABLE_ARTIFACT_PATH)
        .map_err(|error| {
            source_error_from_load(
                &error,
                format!(
                    "read verified A27 design catalog artifact {:?}: {error}",
                    crate::design_catalog::PORTABLE_ARTIFACT_PATH
                ),
            )
        })?;
    bytes
        .map(|bytes| {
            if let Some(recorder_row) = recorder_row {
                let expected_sha = recorder_row
                    .get("sha256")
                    .and_then(Value::as_str)
                    .expect("recorder-owned A27 identity validated above");
                let expected_len = recorder_row
                    .get("bytes")
                    .and_then(Value::as_u64)
                    .expect("recorder-owned A27 identity validated above");
                let actual_sha =
                    snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&bytes));
                if bytes.len() as u64 != expected_len || actual_sha != expected_sha {
                    return Err(SourceError::Open(
                        "verified A27 design-catalog bytes disagree with package authorities"
                            .into(),
                    ));
                }
            }
            crate::design_catalog::PortableDesignCatalog::decode_a27_live_json(&bytes)
                .and_then(|catalog| {
                    catalog.validate_replay_hours(hours)?;
                    Ok(catalog)
                })
                .map(Arc::new)
                .map_err(|error| {
                    SourceError::Open(format!("invalid A27 design catalog artifact: {error}"))
                })
        })
        .transpose()
}

const REQUIRED_PACKAGED_STATIC_ARTIFACTS: &[(&str, &str)] = &[
    (
        "templates_equipment_catalog",
        "static/templates_equipment_catalog/domain59_static_catalog.json",
    ),
    (
        "localisation_join",
        "static/localisation_join/localisation_catalog.json",
    ),
    (
        "decisions_static_definitions",
        "static/decisions_static_definitions/decision_definitions_catalog.json",
    ),
    (
        "idea_tech_loc_xref_static",
        "static/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
    ),
    (
        "ship_definitions_static",
        "static/ship_definitions_static/ship_definitions_catalog.json",
    ),
    (
        "gfx_icon_asset_manifest_static",
        "static/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json",
    ),
    (
        "technology_sharing_static",
        "static/technology_sharing_static/technology_sharing_static.json",
    ),
    (
        "focus_defs_static",
        "static/focus_defs_static/focus_defs_static.json",
    ),
];
const LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT: usize =
    REQUIRED_PACKAGED_STATIC_ARTIFACTS.len() - 1;

/// Outcome of the recorder-owned static-artifact roster join.
struct PackagedStaticArtifactRoster {
    /// Artifacts admitted with their full chain: recorder row, index row,
    /// container entry, payload bytes, and content-manifest epoch.
    admitted: BTreeMap<String, PackagedStaticArtifact>,
    /// Artifacts the package carries and the join REFUSED because neither the
    /// container nor the recorder rows name a finalized portable
    /// content-manifest epoch to bind them to.
    not_content_bound: Vec<String>,
    /// Required artifacts the VERIFIED package does not carry: an index row
    /// disposed `present: false`, or the one artifact a legacy roster predates.
    ///
    /// This is the observed truth. Without it the manifest's declaration was
    /// believed: a capture could declare nothing missing while its index
    /// refused an artifact, and the admission would report the static-artifact
    /// guarantee as held while `static_artifact()` answered `None` (Codex
    /// round 2, MEDIUM).
    absent: BTreeSet<String>,
    /// The artifact a legacy seven-row roster predates, when this package has
    /// one.
    ///
    /// It is held apart from the rest of `absent` because a recorder build
    /// whose required set did not contain `focus_defs_static` truthfully
    /// declared nothing missing while carrying all seven, so reconciling its
    /// `[]` against an absence it had never heard of would refuse a recording
    /// this join promises stays readable (Codex round 3, MEDIUM).
    ///
    /// THE ROSTER SHAPE ALONE DOES NOT ESTABLISH THAT (Codex round 4, HIGH). A
    /// current recorder advertises all eight in
    /// `production_profile.required_static_artifacts`, and one that ships seven
    /// rows while advertising eight is under-declaring, not legacy. The
    /// exemption is therefore applied by
    /// [`MetadataRoute::admit_static_artifacts`], which is where that
    /// advertised set is read, and only when it proves the writer predates the
    /// artifact.
    legacy_skipped: Option<&'static str>,
}

/// The ONE content epoch a recording binds its packaged artifacts to.
///
/// Every consumer that admits a packaged artifact asks this, and only this.
/// There used to be two independent askers — the static-artifact roster and the
/// A34 decision-catalog loader — each reaching straight into
/// `container.content_manifest()`. Teaching only the first about the
/// attach-time epoch left the second refusing the WHOLE recording with
/// "portable content manifest is absent or unfinalized" for the ordinary shape
/// a shipped recorder produces (Codex round 1, HIGH).
enum RecordingContentEpoch {
    /// No content authority at all: nothing may be admitted.
    None,
    /// The container's finalized portable content manifest — the launch-time
    /// epoch, and the only authority a legacy recording could carry.
    ContainerManifest(String),
    /// The KIND C attach-time epoch, RE-DERIVED from the sealed proof.
    ///
    /// The derivation result is carried rather than raised: a recording whose
    /// proof yields no epoch and which packages nothing is simply a recording
    /// with no epoch, and only a row that CLAIMS one turns the reason into a
    /// refusal.
    AttachTime(
        Box<Result<portable_content_manifest::attach_content_epoch::AttachContentEpoch, String>>,
    ),
}

impl RecordingContentEpoch {
    /// The digest every packaged row must carry, if there is one.
    fn digest(&self) -> Option<&str> {
        match self {
            RecordingContentEpoch::None => Option::None,
            RecordingContentEpoch::ContainerManifest(digest) => Some(digest.as_str()),
            RecordingContentEpoch::AttachTime(epoch) => match epoch.as_ref() {
                Ok(epoch) => Some(epoch.digest()),
                Err(_) => Option::None,
            },
        }
    }

    /// Why a declared proof yielded no epoch, when it declared one and failed.
    fn derivation_error(&self) -> Option<&str> {
        match self {
            RecordingContentEpoch::AttachTime(epoch) => match epoch.as_ref() {
                Err(reason) => Some(reason.as_str()),
                Ok(_) => Option::None,
            },
            _ => Option::None,
        }
    }

    /// Whether this epoch authorizes ONE named artifact carrying exactly these
    /// bytes.
    ///
    /// Under the attach-time epoch the join is PER ARTIFACT: one
    /// recording-wide digest says the world was observed, not that THIS catalog
    /// was resolved through it. A launch-time manifest, by contrast, is taken
    /// over the admitted bundle as a whole.
    fn authorizes(&self, role: &str, artifact_sha256: &str) -> Result<(), String> {
        match self {
            RecordingContentEpoch::None => {
                Err("this recording states no content epoch".to_string())
            }
            RecordingContentEpoch::ContainerManifest(_) => Ok(()),
            RecordingContentEpoch::AttachTime(epoch) => match epoch.as_ref() {
                Ok(epoch) => epoch.bind(role, artifact_sha256).map(|_| ()),
                Err(reason) => Err(reason.clone()),
            },
        }
    }
}

/// Resolve a recording's content epoch from its own POSITIVE declaration.
///
/// Exhaustive on the declared kind, never inferred from which field happens to
/// be populated. A late attach carrying `--portable-content-use-trace` has BOTH
/// a container content manifest and an attach-time proof, and a recording that
/// positively declares `none` has no epoch even if a container manifest exists
/// — reading either off field presence is the inference this loader forbids
/// everywhere else (Codex round 1, MEDIUM). Unknown and non-string declarations
/// are refused by the shared validator rather than falling back to anything.
fn recording_content_epoch(
    container: &snapshot::Container,
    recorder_manifest: &Map<String, Value>,
) -> Result<RecordingContentEpoch, SourceError> {
    use portable_content_manifest::attach_content_epoch as epoch_authority;

    let declared =
        epoch_authority::content_proof_kind(recorder_manifest).map_err(SourceError::Open)?;
    let container_digest = container
        .content_manifest()
        .and_then(|manifest| manifest.manifest_digest_sha256.as_deref());
    Ok(match declared {
        // A recording that predates the declaration.
        None => match container_digest {
            Some(digest) => RecordingContentEpoch::ContainerManifest(digest.to_string()),
            None => RecordingContentEpoch::None,
        },
        Some(kind) if kind == epoch_authority::KIND_LAUNCH_TIME_STATIC_BUNDLE_V1 => {
            match container_digest {
                Some(digest) => RecordingContentEpoch::ContainerManifest(digest.to_string()),
                None => {
                    return Err(SourceError::Open(
                        "recording declares the launch-time content epoch but its container \
                         finalized no portable content manifest"
                            .to_string(),
                    ))
                }
            }
        }
        Some(kind) if kind == epoch_authority::KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1 => {
            let block = recorder_manifest
                .get(epoch_authority::ATTACH_TIME_CONTENT_PROOF_FIELD)
                .filter(|value| !value.is_null())
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder manifest declares the attach-time content epoch but carries no \
                         attach_time_content_proof block"
                            .to_string(),
                    )
                })?;
            RecordingContentEpoch::AttachTime(Box::new(epoch_authority::epoch_from_proof_json(
                block,
            )))
        }
        // Positively declared as having none. A container manifest that happens
        // to exist does not become one.
        Some(_) => RecordingContentEpoch::None,
    })
}

/// Consume the complete recorder-owned static-artifact roster at replay
/// admission.  Semantic adapters may select individual catalogs later, but no
/// producer binding is allowed to sit in the portable file unchecked.
///
/// # Which content epoch binds an artifact
///
/// Routed on the recording's own positively-written `content_proof_kind`, never
/// inferred from which field happens to be populated:
///
/// * `launch_time_static_bundle_v1` — the finalized portable content manifest
///   the container carries, built from an effective-VFS observation taken
///   before `ResumeThread`.
/// * `attach_time_effective_vfs_join_v1` — the KIND C attach-time epoch,
///   RE-DERIVED here from the sealed proof block by the one shared authority
///   ([`portable_content_manifest::attach_content_epoch`]) the recorder stamped
///   it with. This is the route every shipped recording takes: the
///   pre-`ResumeThread` actuator is compiled out of `collector_record`, so no
///   product recording can finalize a portable content manifest, and before
///   this route existed EVERY packaged catalog on EVERY real recording was
///   declined. An artifact is admitted only if that proof joined it, and joined
///   the exact bytes the package carries.
/// * `none`, or a legacy recording that declares nothing — the container's
///   content manifest if it has one, otherwise no epoch at all.
///
/// # Recordings with no epoch of any kind
///
/// Such a recording used to abort the whole open, which refused the recording
/// over artifacts it could simply decline.
///
/// It now declines them, and only them. The epoch check is not weakened in
/// either direction: with an epoch, every row must carry exactly it; without
/// one, every row must carry NONE — a row claiming an epoch the container never
/// finalized is a contradiction and still refuses the recording. Unbindable
/// artifacts are then verified for internal consistency and left OUT of the
/// admitted map, so [`ReplaySource::static_artifact`] answers `None` for each
/// and the semantic adapters above it stay unavailable rather than joining a
/// catalog to content nothing proves it came from. What was refused is named in
/// [`RecorderProductAdmission::static_artifacts_not_content_bound`].
fn validate_packaged_static_artifact_roster(
    store: &Store,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
    claims_production_profile: bool,
) -> Result<PackagedStaticArtifactRoster, SourceError> {
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(PackagedStaticArtifactRoster {
            admitted: BTreeMap::new(),
            not_content_bound: Vec::new(),
            absent: BTreeSet::new(),
            legacy_skipped: None,
        });
    };
    let recorder_rows = recorder_manifest
        .get("replay_packaging")
        .and_then(|packaging| packaging.get("static_artifacts"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open(
                "recorder-owned replay has no static-artifact packaging roster".into(),
            )
        })?;
    // EVERY RECORDER ROW MUST BE A RECOGNISED, UNIQUE REQUIRED ARTIFACT. The
    // per-name loop below selects rows by name, so a row named anything else —
    // or a second row for a name that is not required — was matched by nothing
    // and silently ignored, and the container-role equality check could not see
    // it either because it only compares the paths the loop expected. An
    // unreadable authority row in the roster is malformed, not decoration
    // (Codex round 3, MEDIUM).
    {
        let mut seen = BTreeSet::new();
        for (index, row) in recorder_rows.iter().enumerate() {
            let name = row.get("name").and_then(Value::as_str).ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder manifest replay_packaging.static_artifacts[{index}] has no string name"
                ))
            })?;
            if !REQUIRED_PACKAGED_STATIC_ARTIFACTS
                .iter()
                .any(|(required, _)| *required == name)
            {
                return Err(SourceError::Open(format!(
                    "recorder manifest packages static artifact {name:?}, which is not a required \
                     static artifact"
                )));
            }
            if !seen.insert(name) {
                return Err(SourceError::Open(format!(
                    "recorder manifest packages static artifact {name:?} more than once"
                )));
            }
        }
    }
    let index_bytes = store
        .read_game_file_bytes(game_id, "index.json")
        .map_err(|error| {
            let message = format!(
                "read authenticated index for static-artifact roster join: {error}"
            );
            source_error_from_load(&error, message)
        })?
        .ok_or_else(|| {
            SourceError::Open(
                "selected game index disappeared during static-artifact roster join".into(),
            )
        })?;
    let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse selected game index for static-artifact roster join: {error}"
        ))
    })?;
    let index_rows = index
        .get("static_artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open("recorder-owned replay index has no static-artifact roster".into())
        })?;
    let legacy_without_focus_defs =
        index_rows.len() == LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT;
    if !legacy_without_focus_defs
        && index_rows.len() != REQUIRED_PACKAGED_STATIC_ARTIFACTS.len()
    {
        return Err(SourceError::Open(format!(
            "static-artifact index roster has {} rows; expected current {} or legacy {}",
            index_rows.len(),
            REQUIRED_PACKAGED_STATIC_ARTIFACTS.len(),
            LEGACY_PACKAGED_STATIC_ARTIFACT_COUNT,
        )));
    }
    let container = store.container().ok_or_else(|| {
        SourceError::Open("static-artifact roster requires a portable container authority".into())
    })?;
    let epoch_authority = recording_content_epoch(container, recorder_manifest)?;
    let content_digest = epoch_authority.digest();
    // THE CARVE-OUT DOES NOT REACH A CAPTURE THAT CLAIMS THE PROFILE. Building
    // the portable content manifest is one of the things `--profile production`
    // guarantees, so a recording asserting `production_profile.enabled == true`
    // without a finalized epoch is contradicting itself, exactly as it would by
    // naming an unscheduled profile domain. It keeps the original mandatory
    // refusal; only a disabled or unstated authority reaches the declining path
    // below (Codex round 1, HIGH).
    if claims_production_profile && content_digest.is_none() {
        return Err(SourceError::Open(
            "static-artifact roster has no finalized portable content-manifest epoch while \
             recorder production_profile.enabled is true; the production profile is what builds \
             that manifest, so a capture claiming it must carry one"
                .into(),
        ));
    }

    let mut expected_container_paths = BTreeSet::new();
    let mut admitted = BTreeMap::new();
    let mut not_content_bound = Vec::new();
    let mut absent = BTreeSet::new();
    let mut legacy_skipped = None;
    for &(name, expected_path) in REQUIRED_PACKAGED_STATIC_ARTIFACTS {
        let recorder_matches = recorder_rows
            .iter()
            .filter(|row| row.get("name").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>();
        if recorder_matches.len() > 1 {
            return Err(SourceError::Open(format!(
                "recorder manifest declares static artifact {name:?} more than once"
            )));
        }
        let index_matches = index_rows
            .iter()
            .filter(|row| row.get("name").and_then(Value::as_str) == Some(name))
            .collect::<Vec<_>>();
        if legacy_without_focus_defs
            && name == "focus_defs_static"
            && recorder_matches.is_empty()
            && index_matches.is_empty()
        {
            // A pre-focus-carrier recording remains readable but exposes no
            // focus definitions. A31 stays unavailable; there is no local VFS
            // or playback-machine fallback. It is still an artifact the package
            // does not carry, so the guarantee reflects it — but the recorder
            // build that wrote a seven-row roster had no such artifact in its
            // required set, so it is deliberately NOT something that build's
            // declaration could have named.
            absent.insert(name.to_string());
            legacy_skipped = Some(name);
            continue;
        }
        if index_matches.len() != 1 {
            return Err(SourceError::Open(format!(
                "index must declare static artifact {name:?} exactly once"
            )));
        }
        let index_row = index_matches[0];
        let present = index_row.get("present").and_then(Value::as_bool);
        match (present, recorder_matches.as_slice()) {
            (Some(false), []) => {
                absent.insert(name.to_string());
                continue;
            }
            (Some(false), [_]) => {
                return Err(SourceError::Open(format!(
                    "recorder manifest records {name:?} present while index records it absent"
                )))
            }
            (Some(true), [recorder_row]) => {
                let sha = recorder_row.get("sha256").and_then(Value::as_str);
                let bytes = recorder_row.get("bytes").and_then(Value::as_u64);
                // TYPED, not `and_then(as_str)`. A row whose epoch is `7`,
                // `{}`, `[]` or `false` would otherwise read as `None` and be
                // taken for the null disposition the no-epoch branch requires.
                // The recorder writes this field on every row it packs, as a
                // digest string or as JSON null, so an absent one is malformed
                // rather than legacy.
                let epoch = match recorder_row.get("content_manifest_digest_sha256") {
                    Some(Value::Null) => None,
                    Some(Value::String(value)) => Some(value.as_str()),
                    _ => {
                        return Err(SourceError::Open(format!(
                            "recorder manifest static artifact {name:?} \
                             content_manifest_digest_sha256 is absent or is neither a digest \
                             string nor null"
                        )))
                    }
                };
                // With a finalized epoch every row must carry exactly it.
                // Without one every row must carry null: a row naming an epoch
                // the container never finalized is a contradiction, and it
                // refuses the recording here rather than being declined below.
                // A row claiming an epoch this recording's own proof could
                // not produce is a contradiction, and the reason the proof gave
                // is what the refusal says. Reached only for a row that is NOT
                // null, so a recording that packages nothing still opens.
                if let (Some(reason), Some(_)) = (epoch_authority.derivation_error(), epoch) {
                    return Err(SourceError::Open(format!(
                        "recorder manifest static artifact {name:?} claims a content epoch its \
                         own attach-time proof does not yield: {reason}"
                    )));
                }
                let epoch_binds = match content_digest {
                    Some(digest) => epoch == Some(digest),
                    None => epoch.is_none(),
                };
                if recorder_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || sha.is_none_or(|value| !lower_sha256(value))
                    || bytes.is_none()
                    || !epoch_binds
                {
                    return Err(SourceError::Open(format!(
                        "recorder manifest static artifact {name:?} identity is invalid"
                    )));
                }
                let sha = sha.expect("validated static-artifact sha");
                let bytes = bytes.expect("validated static-artifact length");
                if index_row.get("role").and_then(Value::as_str) != Some("static_artifact")
                    || index_row.get("path").and_then(Value::as_str) != Some(expected_path)
                    || index_row.get("sha256").and_then(Value::as_str) != Some(sha)
                    || index_row.get("bytes").and_then(Value::as_u64) != Some(bytes)
                {
                    return Err(SourceError::Open(format!(
                        "index static artifact {name:?} disagrees with recorder manifest"
                    )));
                }
                let container_path = format!("payload/{game_id}/{expected_path}");
                let entries = container
                    .entries_by_role("static_artifact")
                    .into_iter()
                    .filter(|entry| entry.path == container_path)
                    .collect::<Vec<_>>();
                let entry = match entries.as_slice() {
                    [entry] => entry,
                    [] => {
                        return Err(SourceError::Open(format!(
                            "static artifact {name:?} has no matching container role"
                        )))
                    }
                    _ => {
                        return Err(SourceError::Open(format!(
                            "static artifact {name:?} has duplicate container roles"
                        )))
                    }
                };
                if entry.sha256 != sha || entry.bytes != bytes {
                    return Err(SourceError::Open(format!(
                        "container static artifact {name:?} disagrees with recorder/index hashes"
                    )));
                }
                let payload = store
                    .read_game_file_bytes(game_id, expected_path)
                    .map_err(|error| {
                        let message = format!(
                            "read verified static artifact {name:?}: {error}"
                        );
                        source_error_from_load(&error, message)
                    })?
                    .ok_or_else(|| {
                        SourceError::Open(format!(
                            "verified static artifact {name:?} payload is absent"
                        ))
                    })?;
                let actual_sha =
                    snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&payload));
                if payload.len() as u64 != bytes || actual_sha != sha {
                    return Err(SourceError::Open(format!(
                        "verified static artifact {name:?} bytes disagree with package authorities"
                    )));
                }
                // Verified in full above; admitted only when an epoch binds it
                // to the effective content it was derived from.
                // UNDER THE ATTACH-TIME EPOCH THE JOIN IS PER ARTIFACT. One
                // recording-wide digest says the world was observed; it does not
                // say THIS catalog was resolved through it. The proof names the
                // roles it certified and the digest it read for each, so an
                // artifact the proof refused, never joined, or joined in a
                // different build is declined by name rather than admitted on
                // the strength of its neighbours.
                if epoch_authority.authorizes(name, &actual_sha).is_err() {
                    not_content_bound.push(name.to_string());
                    expected_container_paths.insert(container_path);
                    continue;
                }
                match content_digest {
                    Some(digest) => {
                        admitted.insert(
                            name.to_string(),
                            PackagedStaticArtifact {
                                name: name.to_string(),
                                path: expected_path.to_string(),
                                sha256: actual_sha,
                                content_manifest_digest_sha256: digest.to_string(),
                                bytes: Arc::from(payload),
                            },
                        );
                    }
                    None => not_content_bound.push(name.to_string()),
                }
                expected_container_paths.insert(container_path);
            }
            (Some(true), []) => {
                return Err(SourceError::Open(format!(
                    "index records static artifact {name:?} present but recorder manifest omits it"
                )))
            }
            _ => {
                return Err(SourceError::Open(format!(
                    "index static artifact {name:?} has no boolean present disposition"
                )))
            }
        }
    }
    let actual_container_paths = container
        .entries_by_role("static_artifact")
        .into_iter()
        .map(|entry| entry.path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if actual_container_paths != expected_container_paths {
        return Err(SourceError::Open(
            "container static-artifact roles do not exactly match the recorder/index roster"
                .into(),
        ));
    }
    Ok(PackagedStaticArtifactRoster {
        admitted,
        not_content_bound,
        absent,
        legacy_skipped,
    })
}

const DECISION_CATALOG_NAME: &str = "decisions_static_definitions";
const DECISION_CATALOG_PATH: &str =
    "static/decisions_static_definitions/decision_definitions_catalog.json";

fn lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Admit A34's immutable effective decision catalog without any machine-local
/// fallback. A usable artifact must be named identically by all four package
/// authorities:
///
/// 1. recorder_manifest.replay_packaging.static_artifacts;
/// 2. the selected game's authenticated index static-artifact roster;
/// 3. the portable content-manifest digest sealed into the recorder row; and
/// 4. the container manifest entry whose verified bytes are exposed by Store.
///
/// Legacy packages that predate the content-manifest digest field leave A34
/// explicitly unavailable. Contradictory declarations are corruption and
/// reject the open.
fn read_packaged_decision_catalog(
    store: &Store,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<Option<PackagedDecisionCatalog>, SourceError> {
    let recorder_rows = recorder_manifest
        .and_then(|manifest| manifest.get("replay_packaging"))
        .and_then(|packaging| packaging.get("static_artifacts"))
        .and_then(Value::as_array);
    let recorder_matches: Vec<&Value> = recorder_rows
        .into_iter()
        .flatten()
        .filter(|row| row.get("name").and_then(Value::as_str) == Some(DECISION_CATALOG_NAME))
        .collect();
    if recorder_matches.len() > 1 {
        return Err(SourceError::Open(
            "recorder manifest declares the A34 decision catalog more than once".into(),
        ));
    }

    let index_bytes = store
        .read_game_file_bytes(game_id, "index.json")
        .map_err(|error| {
            let message = format!(
                "read authenticated index for A34 decision catalog join: {error}"
            );
            source_error_from_load(&error, message)
        })?
        .ok_or_else(|| {
            SourceError::Open(
                "selected game index disappeared during A34 decision catalog join".into(),
            )
        })?;
    let index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse selected game index for A34 decision catalog join: {error}"
        ))
    })?;
    let index_matches: Vec<&Value> = index
        .get("static_artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("name").and_then(Value::as_str) == Some(DECISION_CATALOG_NAME))
        .collect();
    if index_matches.len() > 1 {
        return Err(SourceError::Open(
            "selected game index declares the A34 decision catalog more than once".into(),
        ));
    }
    let Some(index_row) = index_matches.first().copied() else {
        if recorder_matches.is_empty() {
            return Ok(None);
        }
        return Err(SourceError::Open(
            "recorder manifest declares A34 decision catalog but index roster omits it".into(),
        ));
    };
    if index_row.get("present").and_then(Value::as_bool) != Some(true) {
        if recorder_matches.is_empty() {
            return Ok(None);
        }
        return Err(SourceError::Open(
            "recorder manifest declares A34 decision catalog but index records it absent".into(),
        ));
    }
    // Snapshot-only and legacy directory stores may have an unauthenticated
    // index roster but no recorder manifest at all. They remain playable with
    // XUI19 unavailable; there is no recorder claim to contradict.
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(None);
    };
    let recorder_row = recorder_matches.first().copied().ok_or_else(|| {
        SourceError::Open(
            "index records A34 decision catalog present but recorder manifest omits it".into(),
        )
    })?;

    let recorder_path = recorder_row.get("path").and_then(Value::as_str);
    let recorder_sha = recorder_row.get("sha256").and_then(Value::as_str);
    let recorder_len = recorder_row.get("bytes").and_then(Value::as_u64);
    let recorder_content_digest = recorder_row
        .get("content_manifest_digest_sha256")
        .and_then(Value::as_str);
    // The field was introduced with the production A34 package join. Old
    // packages remain playable, but this feature is unavailable rather than
    // consulting a local generated catalog.
    let Some(recorder_content_digest) = recorder_content_digest else {
        return Ok(None);
    };
    if recorder_path != Some(DECISION_CATALOG_PATH)
        || recorder_sha.is_none_or(|value| !lower_sha256(value))
        || recorder_len.is_none()
        || !lower_sha256(recorder_content_digest)
    {
        return Err(SourceError::Open(
            "recorder manifest A34 decision catalog identity is invalid".into(),
        ));
    }
    let recorder_sha = recorder_sha.expect("validated recorder A34 sha");
    let recorder_len = recorder_len.expect("validated recorder A34 length");

    let index_path = index_row.get("path").and_then(Value::as_str);
    let index_sha = index_row.get("sha256").and_then(Value::as_str);
    let index_len = index_row.get("bytes").and_then(Value::as_u64);
    if index_row.get("role").and_then(Value::as_str) != Some("static_artifact")
        || index_path != Some(DECISION_CATALOG_PATH)
        || index_sha != Some(recorder_sha)
        || index_len != Some(recorder_len)
    {
        return Err(SourceError::Open(
            "index A34 decision catalog identity disagrees with recorder manifest".into(),
        ));
    }

    let container = store.container().ok_or_else(|| {
        SourceError::Open(
            "A34 decision catalog requires a portable container content-manifest authority"
                .into(),
        )
    })?;
    // THE SAME AUTHORITY THE STATIC-ARTIFACT ROSTER ASKS. This used to reach
    // straight into `container.content_manifest()`, which no shipped recording
    // can carry, so the ordinary attach-time shape — a row stamped with a real
    // KIND C epoch — refused the WHOLE recording here after the roster had
    // already admitted the same artifact (Codex round 1, HIGH).
    let epoch_authority = recording_content_epoch(container, recorder_manifest)?;
    let Some(content_digest) = epoch_authority.digest() else {
        // No epoch of any kind: decline the catalog exactly as the roster
        // declines an unbound artifact. XUI19 is unavailable and the recording
        // still opens.
        return Ok(None);
    };
    if content_digest != recorder_content_digest {
        return Err(SourceError::Open(
            "A34 decision catalog recorder row is not bound to the selected content manifest"
                .into(),
        ));
    }
    // And under the attach-time epoch the proof must have joined THIS catalog,
    // at these exact bytes — not merely observed the world it came from.
    if epoch_authority
        .authorizes(DECISION_CATALOG_NAME, recorder_sha)
        .is_err()
    {
        return Ok(None);
    }

    let container_path = format!("payload/{game_id}/{DECISION_CATALOG_PATH}");
    let entries: Vec<_> = container
        .entries_by_role("static_artifact")
        .into_iter()
        .filter(|entry| entry.path == container_path)
        .collect();
    let entry = match entries.as_slice() {
        [entry] => entry,
        [] => {
            return Err(SourceError::Open(
                "A34 decision catalog index row has no matching container-manifest entry".into(),
            ))
        }
        _ => {
            return Err(SourceError::Open(
                "A34 decision catalog has duplicate container-manifest entries".into(),
            ))
        }
    };
    if entry.sha256 != recorder_sha || entry.bytes != recorder_len {
        return Err(SourceError::Open(
            "A34 decision catalog container entry disagrees with recorder/index hashes".into(),
        ));
    }

    let bytes = store
        .read_game_file_bytes(game_id, DECISION_CATALOG_PATH)
        .map_err(|error| {
            let message = format!(
                "read verified A34 decision catalog payload: {error}"
            );
            source_error_from_load(&error, message)
        })?
        .ok_or_else(|| SourceError::Open("verified A34 decision catalog payload is absent".into()))?;
    let actual_sha =
        snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(bytes.as_slice()));
    if bytes.len() as u64 != recorder_len || actual_sha != recorder_sha {
        return Err(SourceError::Open(
            "verified A34 decision catalog bytes disagree with package authorities".into(),
        ));
    }

    Ok(Some(PackagedDecisionCatalog {
        path: DECISION_CATALOG_PATH.into(),
        sha256: actual_sha,
        content_manifest_digest_sha256: content_digest.into(),
        bytes: Arc::from(bytes),
    }))
}

fn validate_index_schema_version(version: Option<&Value>) -> Result<(), SourceError> {
    match version {
        // The canonical JSON writer uses a string. Numeric 1 is retained for
        // legacy directory stores that predate the canonical string spelling.
        None => Ok(()),
        Some(Value::String(value)) if value == "1" => Ok(()),
        Some(Value::Number(value)) if value.as_u64() == Some(1) => Ok(()),
        Some(value) => Err(SourceError::Unsupported(format!(
            "incompatible replay index schema_version {value}; this standalone replayer supports schema_version \"1\""
        ))),
    }
}

fn same_supported_schema_version(left: &Value, right: &Value) -> bool {
    fn is_v1(value: &Value) -> bool {
        matches!(value, Value::String(text) if text == "1")
            || matches!(value, Value::Number(number) if number.as_u64() == Some(1))
    }
    is_v1(left) && is_v1(right)
}

/// Require the recorder-owned content authority to be the exact authority the
/// container reader exposed to playback.  Both objects are individually
/// digest-validated, but without this join a package could carry two valid,
/// different manifests and the replay would silently follow only the outer
/// container copy.
fn validate_recorder_content_manifest_join(
    recorder_session_manifest: &Map<String, Value>,
    container_manifest: Option<&portable_content_manifest::ContentManifest>,
) -> Result<(), SourceError> {
    let recorder_value = recorder_session_manifest.get("portable_content_manifest");
    match (recorder_value, container_manifest) {
        (None | Some(Value::Null), None) => Ok(()),
        (Some(Value::Object(_)), Some(container_manifest)) => {
            let recorder_manifest: portable_content_manifest::ContentManifest =
                serde_json::from_value(
                    recorder_value
                        .expect("object arm has a recorder content manifest")
                        .clone(),
                )
                .map_err(|error| {
                    SourceError::Open(format!(
                        "recorder portable_content_manifest is malformed: {error}"
                    ))
                })?;
            recorder_manifest.validate().map_err(|error| {
                SourceError::Open(format!(
                    "recorder portable_content_manifest failed validation: {error}"
                ))
            })?;
            if &recorder_manifest != container_manifest {
                return Err(SourceError::Open(
                    "recorder portable_content_manifest disagrees with container content_manifest"
                        .into(),
                ));
            }
            let modset = recorder_session_manifest
                .get("modset")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound modset authority".into(),
                    )
                })?;
            let recorder_executable = modset
                .get("game")
                .and_then(Value::as_object)
                .and_then(|game| game.get("exe_sha256"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound executable SHA-256".into(),
                    )
                })?;
            if recorder_executable != container_manifest.source_game.executable_sha256 {
                return Err(SourceError::Open(
                    "recorder modset executable disagrees with portable content source game"
                        .into(),
                ));
            }
            // The launch settings live in their own binding: a resolved
            // CONTENT chain no longer implies the settings were proven, and
            // `settings_binding.authority` states which it is.
            let settings_binding = modset.get("settings_binding").ok_or_else(|| {
                SourceError::Open(
                    "recorder portable content has no modset settings binding".into(),
                )
            })?;
            let recorder_language = settings_binding
                .get("language")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound localisation language"
                            .into(),
                    )
                })?;
            if recorder_language != container_manifest.localisation_language {
                return Err(SourceError::Open(
                    "recorder modset language disagrees with portable content language".into(),
                ));
            }
            let display: RecorderDisplayVariant = serde_json::from_value(
                recorder_display_variant_value(settings_binding).ok_or_else(|| {
                    SourceError::Open(
                        "recorder portable content has no launch-bound display variant".into(),
                    )
                })?,
            )
            .map_err(|error| {
                SourceError::Open(format!(
                    "recorder portable-content display variant is malformed: {error}"
                ))
            })?;
            let expected_settings = BTreeMap::from([
                ("physical_width".into(), display.physical_width.to_string()),
                ("physical_height".into(), display.physical_height.to_string()),
                ("min_gui_width".into(), display.min_gui_width.to_string()),
                ("min_gui_height".into(), display.min_gui_height.to_string()),
                (
                    "gui_scale_f32_bits".into(),
                    display.gui_scale_f32_bits.to_string(),
                ),
                (
                    "refresh_rate_hz".into(),
                    display.refresh_rate_hz.to_string(),
                ),
                (
                    "max_refresh_rate_hz".into(),
                    display
                        .max_refresh_rate_hz
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unavailable".into()),
                ),
                ("fullscreen".into(), display.fullscreen.to_string()),
                ("borderless".into(), display.borderless.to_string()),
                ("display_mode".into(), display.display_mode),
                (
                    "settings_file_sha256".into(),
                    display.settings_file_sha256,
                ),
                // HOW WELL the settings above are proven travels in the same
                // map that carries them, so a replay reproducing them cannot
                // miss that a recording only OBSERVED the disk while the game
                // was running rather than proving the game read it.
                (
                    "settings_authority".into(),
                    settings_binding
                        .get("authority")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            SourceError::Open(
                                "recorder portable content states no settings authority".into(),
                            )
                        })?
                        .to_string(),
                ),
            ]);
            if container_manifest.required_settings != expected_settings {
                return Err(SourceError::Open(
                    "recorder display variant disagrees with portable content required settings"
                        .into(),
                ));
            }
            Ok(())
        }
        (None | Some(Value::Null), Some(_)) => Err(SourceError::Open(
            "container declares portable content but recorder manifest omits its matching authority"
                .into(),
        )),
        (Some(value), Some(_)) => Err(SourceError::Open(format!(
            "recorder portable_content_manifest must be an object when the container declares one; found {value}"
        ))),
        (Some(_), None) => Err(SourceError::Open(
            "recorder declares portable content but container manifest omits it".into(),
        )),
    }
}

/// Reconcile the recorder's portable-file declarations before selecting either
/// the snapshot or compact-prefold playback path.
///
/// Legacy directory fixtures do not have a container manifest and remain
/// covered by their existing compatibility path.  A recorder-owned portable
/// container, however, must present one exact role/schema graph: manifest and
/// index schema v1, one canonical index role, and a one-to-one index ↔
/// role:snapshot mapping including hour metadata.  This prevents an entry from
/// being selected by path while its declared role says something else.
fn validate_recorder_portable_container_contract(
    store: &Store,
    game_id: &str,
    index: &snapshot::Index,
    metadata: &MetadataRoute,
) -> Result<(), SourceError> {
    let Some(recorder_manifest) = metadata.recorder_manifest.as_ref() else {
        return Ok(());
    };
    let container = store.container().ok_or_else(|| {
        SourceError::Open(
            "recorder-owned replay metadata requires a portable container authority".into(),
        )
    })?;
    validate_recorder_content_manifest_join(recorder_manifest, container.content_manifest())?;
    if let Some(declared_game_id) = recorder_manifest.get("replay_game_id") {
        if declared_game_id.as_str() != Some(game_id) {
            return Err(SourceError::Open(format!(
                "recorder replay_game_id {declared_game_id} disagrees with selected container game_id {game_id:?}"
            )));
        }
    }

    let manifest_schema = container
        .manifest()
        .get("schema_version")
        .ok_or_else(|| {
            SourceError::Open(
                "recorder-owned container manifest has no schema_version authority".into(),
            )
        })?;
    let index_schema = index.schema_version.as_ref().ok_or_else(|| {
        SourceError::Open("recorder-owned replay index has no schema_version authority".into())
    })?;
    if !same_supported_schema_version(manifest_schema, index_schema) {
        return Err(SourceError::Open(format!(
            "recorder-owned container manifest schema_version {manifest_schema} disagrees with index schema_version {index_schema}"
        )));
    }

    let canonical_index = format!("payload/{game_id}/index.json");
    let index_roles = container.entries_by_role("index");
    if index_roles.len() != 1 || index_roles[0].path != canonical_index {
        return Err(SourceError::Open(format!(
            "recorder-owned replay requires exactly one canonical index role {canonical_index:?}; found {:?}",
            index_roles
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>()
        )));
    }
    let index_bytes = container
        .read_entry_bytes(&canonical_index)
        .map_err(|error| {
            let message = format!(
                "read canonical index for recorder fold-disposition join: {error}"
            );
            source_error_from_container(&error, message)
        })?;
    let raw_index = snapshot::parse_json_strict(&index_bytes).map_err(|error| {
        SourceError::Open(format!(
            "strict parse canonical index for recorder fold-disposition join: {error}"
        ))
    })?;
    let binary_replay_fold = raw_index
        .get("replay_decoder")
        .is_some_and(Value::is_object)
        && raw_index
            .get("production_json_snapshots_omitted")
            .and_then(Value::as_bool)
            == Some(true);
    if let Some(disposition) = raw_index.get("host_fold_skipped") {
        let skipped = disposition.as_bool().ok_or_else(|| {
            SourceError::Open(
                "recorder-owned index host_fold_skipped must be a boolean".into(),
            )
        })?;
        let expected_skipped = index.snapshots.is_empty() && !binary_replay_fold;
        if skipped != expected_skipped {
            return Err(SourceError::Open(format!(
                "recorder-owned index host_fold_skipped={skipped} contradicts snapshot count {} and binary replay fold={binary_replay_fold}",
                index.snapshots.len(),
            )));
        }
    }

    let snapshot_roles = container.entries_by_role("snapshot");
    if snapshot_roles.len() != index.snapshots.len() {
        return Err(SourceError::Open(format!(
            "recorder-owned replay index lists {} snapshots but container declares {} snapshot roles",
            index.snapshots.len(),
            snapshot_roles.len()
        )));
    }
    for indexed in &index.snapshots {
        let expected_path = format!("payload/{game_id}/{}", indexed.file);
        let matches = snapshot_roles
            .iter()
            .filter(|entry| entry.path == expected_path)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(SourceError::Open(format!(
                "indexed snapshot {:?} requires exactly one matching snapshot role; found {}",
                indexed.file,
                matches.len()
            )));
        }
        let snapshot_bytes = container
            .read_entry_bytes(&matches[0].path)
            .map_err(|error| {
                let message = format!(
                    "read indexed snapshot {:?} for hour binding: {error}",
                    matches[0].path
                );
                source_error_from_container(&error, message)
            })?;
        let snapshot_value = snapshot::parse_json_strict(&snapshot_bytes).map_err(|error| {
            SourceError::Open(format!(
                "strict parse indexed snapshot {:?} for hour binding: {error}",
                matches[0].path
            ))
        })?;
        let body_hour = snapshot_value.get("hour_index").and_then(Value::as_i64);
        if body_hour != indexed.hour_index {
            return Err(SourceError::Open(format!(
                "snapshot role {:?} hour_index {:?} disagrees with index hour_index {:?}",
                matches[0].path, body_hour, indexed.hour_index
            )));
        }
    }

    let timeline = metadata
        .timeline
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open("recorder-owned replay has no timeline object".into())
        })?;
    for (field, expected) in [
        ("format", COMPACT_STREAM_TIMELINE_FORMAT),
        ("stream_format", "compact"),
        ("raw_stream_role", COMPACT_STREAM_ROLE),
        ("session_manifest_role", "recorder_manifest"),
    ] {
        if timeline.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(SourceError::Open(format!(
                "recorder-owned timeline {field} must be {expected:?}"
            )));
        }
    }
    // The recording states its composition in more than one document, so those
    // statements have to agree. A timeline that claims a composition the
    // recorder manifest does not is a rewritten recording, not a readable one.
    let manifest_packaging = declared_raw_stream_packaging(Some(recorder_manifest))?;
    let timeline_packaging = snapshot::read_raw_stream_packaging_with_legacy_default(timeline)
        .map_err(|error| {
            SourceError::Open(format!("recorder-owned timeline composition: {error}"))
        })?;
    if manifest_packaging != timeline_packaging {
        return Err(SourceError::Open(format!(
            "recorder-owned timeline declares the {:?} composition but the recorder manifest \
             declares {:?}",
            timeline_packaging.as_str(),
            manifest_packaging.as_str(),
        )));
    }
    // index.json is the third document that states the composition. It is
    // compared HERE rather than only in the cache-backed admission, because
    // this contract runs for EVERY recorder-owned recording — snapshot-flavor
    // and raw-stream-decoder routes included — and those routes would otherwise
    // never look at it.
    let index_packaging = raw_index
        .as_object()
        .ok_or_else(|| {
            SourceError::Open("recorder-owned replay index is not an object".to_string())
        })
        .and_then(|object| {
            snapshot::read_raw_stream_packaging_with_legacy_default(object).map_err(|error| {
                SourceError::Open(format!("recorder-owned index descriptor composition: {error}"))
            })
        })?;
    if index_packaging != manifest_packaging {
        return Err(SourceError::Open(format!(
            "recorder-owned index descriptor declares the {:?} composition but the recorder \
             manifest declares {:?}",
            index_packaging.as_str(),
            manifest_packaging.as_str(),
        )));
    }
    // A cache-only recording must not NAME a stream path in any of its
    // documents, and must not CARRY one either — whichever route will go on to
    // serve it. The cache-backed admission already checks the physical entry
    // against the declaration; doing it here too covers the snapshot-flavor and
    // raw-decoder routes, which otherwise compare the three declarations to
    // each other and never to the container.
    //
    // Both directions are reconciled, because the snapshot-flavor route serves
    // its recording straight from `StoreSource` and would otherwise compare the
    // three declarations only to each other, never to the container.
    let stream_entries = container.entries_by_role(snapshot::COMPACT_STREAM_ROLE);
    match (manifest_packaging, stream_entries.len()) {
        (snapshot::RawStreamPackaging::Packaged, 1) => {
            // The cache and raw-decoder routes bind the packaged entry to the
            // timeline path and to the recorder's capture receipt. The
            // snapshot-flavor route goes straight to `StoreSource` and would
            // otherwise apply neither, so a packaged snapshot recording could
            // carry a stream role whose path or identity contradicted its own
            // documents. Bind it here, where every route passes.
            let entry = &stream_entries[0];
            validate_compact_stream_entry_identity(
                &entry.path,
                game_id,
                metadata.timeline.as_ref(),
            )?;
            validate_recorder_compact_stream_join(
                &entry.path,
                entry.bytes,
                &entry.sha256,
                game_id,
                Some(recorder_manifest),
            )?;
        }
        (snapshot::RawStreamPackaging::Omitted, 0) => {
            validate_no_stream_path_is_claimed(
                Some(recorder_manifest),
                metadata.timeline.as_ref(),
            )?;
        }
        (packaging, count) => {
            return Err(SourceError::Open(format!(
                "recorder-owned recording declares the {:?} composition but the container carries \
                 {count} entries with role {:?}",
                packaging.as_str(),
                snapshot::COMPACT_STREAM_ROLE,
            )))
        }
    }
    Ok(())
}

/// Bind the recorder's capture-time compact-stream authority to the exact role
/// entry selected for prefolding. Older recorder manifests may omit these
/// objects; once either object exists, all of its current identity fields are
/// required and must agree with the authenticated container entry.
fn validate_recorder_compact_stream_join(
    entry_path: &str,
    entry_bytes: u64,
    entry_sha256: &str,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<(), SourceError> {
    let Some(recorder_manifest) = recorder_manifest else {
        return Ok(());
    };
    let relative_path = entry_path
        .strip_prefix(&format!("payload/{game_id}/"))
        .ok_or_else(|| {
            SourceError::Open(format!(
                "compact stream role entry {entry_path:?} is outside selected game {game_id:?}"
            ))
        })?;

    if let Some(storage_value) = recorder_manifest.get("stream_storage") {
        let storage = storage_value.as_object().ok_or_else(|| {
            SourceError::Open("recorder stream_storage authority is not an object".into())
        })?;
        if storage.get("decoded_bytes").and_then(Value::as_u64) != Some(entry_bytes)
            || storage.get("decoded_sha256").and_then(Value::as_str) != Some(entry_sha256)
        {
            return Err(SourceError::Open(
                "recorder stream_storage decoded identity disagrees with selected compact stream role"
                    .into(),
            ));
        }
    }

    if let Some(entries_value) = recorder_manifest.get("replay_entries") {
        let entries = entries_value.as_object().ok_or_else(|| {
            SourceError::Open("recorder replay_entries authority is not an object".into())
        })?;
        if entries.get("stream").and_then(Value::as_str) != Some(relative_path)
            || entries.get("stream_role").and_then(Value::as_str)
                != Some(snapshot::COMPACT_STREAM_ROLE)
        {
            return Err(SourceError::Open(
                "recorder replay_entries stream binding disagrees with selected compact stream role"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn recorder_metadata_route(store: &Store, game_id: &str) -> Result<MetadataRoute, SourceError> {
    if let Some(container) = store.container() {
        let timeline_path = format!("payload/{game_id}/timeline.json");
        let timeline_roles = container.entries_by_role("timeline");
        if timeline_roles.len() > 1 {
            return Err(SourceError::Open(format!(
                "timeline role reconciliation found multiple entries: {:?}",
                timeline_roles
                    .iter()
                    .map(|entry| entry.path.as_str())
                    .collect::<Vec<_>>()
            )));
        }
        if let Some(entry) = timeline_roles.first() {
            if entry.path != timeline_path {
                return Err(SourceError::Open(format!(
                    "timeline role entry {:?} does not byte-match canonical {timeline_path:?}",
                    entry.path
                )));
            }
        }

        let recorder_roles = container.entries_by_role("recorder_manifest");
        let timeline_listed = container
            .entries()
            .iter()
            .any(|entry| entry.path == timeline_path);
        if !timeline_listed {
            if recorder_roles.is_empty() {
                return Ok(MetadataRoute {
                    binding: None,
                    timeline: None,
                    recorder_manifest: None,
                            });
            }
            return Err(SourceError::Open(format!(
                "recorder_manifest role entry exists but canonical timeline {timeline_path:?} is absent"
            )));
        }

        let timeline_bytes = container.read_entry_bytes(&timeline_path).map_err(|error| {
            {
                let message = format!(
                "listed canonical timeline {timeline_path:?} could not be read and verified: {error}"
            );
                source_error_from_container(&error, message)
            }
        })?;
        let timeline = parse_metadata_object(&timeline_bytes, "timeline.json")?;
        let reference = session_manifest_reference(&timeline)?;
        let Some(reference) = reference else {
            if recorder_roles.is_empty() {
                return Ok(MetadataRoute {
                    binding: None,
                    timeline: Some(Value::Object(timeline)),
                    recorder_manifest: None,
                            });
            }
            return Err(SourceError::Open(
                "timeline.json.session_manifest is absent but recorder_manifest role entry exists"
                    .into(),
            ));
        };

        if recorder_roles.len() != 1 {
            return Err(SourceError::Open(format!(
                "timeline.json.session_manifest {reference:?} requires exactly one recorder_manifest role entry; found {}",
                recorder_roles.len()
            )));
        }
        let expected_path = format!("payload/{game_id}/{reference}");
        let entry = &recorder_roles[0];
        if entry.path != expected_path {
            return Err(SourceError::Open(format!(
                "timeline.json.session_manifest {reference:?} expects recorder_manifest path {expected_path:?}, found {:?}",
                entry.path
            )));
        }
        let manifest_bytes = container.read_entry_bytes(&entry.path).map_err(|error| {
            {
                let message = format!(
                "read recorder_manifest {:?} referenced by timeline.json.session_manifest: {error}",
                entry.path
            );
                source_error_from_container(&error, message)
            }
        })?;
        let recorder_manifest =
            parse_metadata_object(&manifest_bytes, &format!("recorder manifest {reference:?}"))?;
        let product_admission = validate_recorder_product_admission(&recorder_manifest)?;
        let binding =
            recorder_manifest_binding_value(&recorder_manifest, &reference, product_admission)?;
        return Ok(MetadataRoute {
            binding: Some(binding),
            timeline: Some(Value::Object(timeline)),
            recorder_manifest: Some(recorder_manifest),
            });
    }

    directory_recorder_metadata_route_with_reader(|relative| {
        store
            .read_game_file_bytes(game_id, relative)
            .map_err(|error| {
                let message = format!(
                    "handle-relative read of {game_id:?}/{relative}: {error}"
                );
                source_error_from_load(&error, message)
            })
    })
}

/// Directory metadata classification and binding over a read-once object API.
/// The production closure performs one handle-relative final-handle read; the
/// counting test uses this same route and makes any second read observable.
fn directory_recorder_metadata_route_with_reader<F>(
    mut read_once: F,
) -> Result<MetadataRoute, SourceError>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>, SourceError>,
{
    let timeline =
        read_optional_metadata_object_once(|| read_once("timeline.json"), "timeline.json")?;
    let Some(timeline) = timeline else {
        return Ok(MetadataRoute {
            binding: None,
            timeline: None,
            recorder_manifest: None,
            });
    };
    let Some(reference) = session_manifest_reference(&timeline)? else {
        return Ok(MetadataRoute {
            binding: None,
            timeline: Some(Value::Object(timeline)),
            recorder_manifest: None,
            });
    };
    let manifest = read_optional_metadata_object_once(
        || read_once(&reference),
        &format!("recorder manifest {reference:?}"),
    )?
    .ok_or_else(|| {
        SourceError::Open(format!(
            "timeline.json.session_manifest references missing recorder manifest {reference:?}"
        ))
    })?;
    let product_admission = validate_recorder_product_admission(&manifest)?;
    let binding = recorder_manifest_binding_value(&manifest, &reference, product_admission)?;
    Ok(MetadataRoute {
        binding: Some(binding),
        timeline: Some(Value::Object(timeline)),
        recorder_manifest: Some(manifest),
    })
}

fn parse_metadata_object(bytes: &[u8], context: &str) -> Result<Map<String, Value>, SourceError> {
    let value = snapshot::parse_json_strict(bytes)
        .map_err(|error| SourceError::Open(format!("strict parse {context}: {error}")))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| SourceError::Open(format!("{context} must decode as a JSON object")))
}

/// Reject recorder manifests that explicitly describe a recovery/quarantined
/// artifact, or a capture whose named shortfall is not one a capture is allowed
/// to have. Missing fields remain legacy compatibility; once an authority is
/// present it must positively admit the artifact rather than relying on
/// defaults.
///
/// # The late-attach admission (owner ruling, 2026-08-14)
///
/// This used to refuse any capture whose `production_profile.enabled` was not
/// `true`. That refused EVERY capture the shipped product can make, and every
/// committed capture in the tree, for a reason none of them can act on:
/// `--profile production` is early-launch-only. Its finalizer builds the
/// portable content manifest from the effective-VFS content observation
/// authenticated in-process before `ResumeThread`, so it requires the
/// in-memory early-launch manifest and the authenticated early process receipt
/// (`recorder_host/src/main.rs`, the `cfg.production` content-manifest arm) —
/// neither of which exists on a `--pid` attach. The shipped product is a late
/// attach and deliberately does not run the profile at all
/// (`recorder_host/src/main.rs::product_auto_attach_worker_args`: "Do not pass
/// `--profile production` here. That profile additionally requires a
/// process-bound, pre-injection static bundle. The ordinary late-attach product
/// cannot fabricate or rebind that evidence."). Capture is attach-and-play, so
/// the gate as written refused the product it exists to admit.
///
/// The carve-out is reasoned rather than blanket, and it is exactly three
/// rules.
///
/// 1. WHAT MAY BE MISSING IS AN ABSENCE, AND AN ABSENCE IS ACCOUNTED FOR, NOT
///    FATAL. `profile_domains_not_scheduled` names domains this capture never
///    scheduled and `missing_required_static_artifacts` names artifacts it
///    never carried. Both are already first-class absences downstream — an
///    unscheduled domain reads `DomainAbsent` at every hour, and an absent
///    static artifact answers `None` from [`ReplaySource::static_artifact`] and
///    is never fabricated. Destroying the whole recording over an accounted
///    absence is the thing `AGENTS.md` forbids: "A bad hour costs that hour
///    with a named reason — never the recording."
/// 2. WHAT MAY NOT BE MISSING IS CORRECTNESS.
///    `mandatory_domains_not_production_complete` is not a scope fact about
///    this capture at all — the recorder computes it from the descriptor
///    STATUSES its own build carries
///    (`compact_contract::mandatory_profile_domains_not_production_complete`).
///    No attach mode explains it, and it says the data the capture DID deliver
///    for those domains is not production data. It refuses, enabled or not.
/// 3. A CAPTURE THAT CLAIMS THE PROFILE IS HELD TO ALL OF IT. When
///    `enabled == true` every list must be empty, exactly as before. The
///    carve-out only ever widens the `enabled == false` case.
///
/// Fail-closed on everything else: `enabled` must be present and boolean (an
/// absent or non-boolean value is a malformed authority, not a false one), each
/// of the three lists must be present and an array of strings, and the returned
/// [`RecorderProductAdmission`] carries the named shortfall into the opened
/// recording's provenance so no consumer has to rediscover it.
fn validate_recorder_product_admission(
    manifest: &Map<String, Value>,
) -> Result<RecorderProductAdmission, SourceError> {
    if let Some(value) = manifest.get("replay_artifact") {
        let artifact = value.as_object().ok_or_else(|| {
            SourceError::Open("recorder replay_artifact authority is not an object".into())
        })?;
        if artifact.get("recovery").and_then(Value::as_bool) != Some(false)
            || artifact.get("reason").and_then(Value::as_str) != Some("gate_passed")
            || !artifact
                .get("gate_problems")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        {
            return Err(SourceError::Open(
                "recorder replay_artifact is recovery-only or did not positively pass its gate"
                    .into(),
            ));
        }
    }
    let Some(value) = manifest.get("production_profile") else {
        return Ok(RecorderProductAdmission::unstated());
    };
    let profile = value.as_object().ok_or_else(|| {
        SourceError::Open("recorder production_profile authority is not an object".into())
    })?;
    let enabled = profile
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            SourceError::Open(
                "recorder production_profile.enabled is absent or is not a boolean".into(),
            )
        })?;
    let read_list = |field: &str| -> Result<Vec<String>, SourceError> {
        let rows = profile.get(field).and_then(Value::as_array).ok_or_else(|| {
            SourceError::Open(format!(
                "recorder production_profile.{field} is absent or is not an array"
            ))
        })?;
        rows.iter()
            .enumerate()
            .map(|(index, row)| {
                row.as_str().map(str::to_string).ok_or_else(|| {
                    SourceError::Open(format!(
                        "recorder production_profile.{field}[{index}] is not a string"
                    ))
                })
            })
            .collect()
    };
    // Read all three before judging any: a malformed list is a malformed
    // authority whichever branch would have looked at it.
    let mandatory_domains_not_production_complete =
        read_list("mandatory_domains_not_production_complete")?;
    let profile_domains_not_scheduled = read_list("profile_domains_not_scheduled")?;
    let missing_required_static_artifacts = read_list("missing_required_static_artifacts")?;

    // CHECK THE DECLARED DIFFERENCE AGAINST THE ROSTERS IT IS A DIFFERENCE OF,
    // AS FAR AS REPLAY CAN.
    //
    // `profile_domains_not_scheduled` was believed outright, so a manifest
    // could name a profile domain, omit it from `scheduled_domains`, and still
    // declare nothing unscheduled — reporting the full schedule as held while
    // the domain read `DomainAbsent` at every hour (Codex round 4, HIGH).
    //
    // THE LIMIT, AND WHY IT IS A LIMIT RATHER THAN A CHOICE. The recorder
    // computes its own difference by RESOLVED DOMAIN ID, and `scheduled_domains`
    // is the operator's vocabulary verbatim, which admits legacy aliases —
    // `state_ownership` resolves to the same domain as the profile's
    // `map_control` (Codex round 5, HIGH). Replay cannot resolve them: the
    // replay workspace takes no dependency on the extractor's domain registry
    // (`game_ui/tests/replay_dependency_boundary.rs`), and re-deriving that
    // table here is exactly the harness drift the project forbids. So the
    // set-difference `profile_domains - scheduled_domains` is NOT computable
    // here without inventing a domain as missing and refusing a legitimate
    // full-profile recording — a false refusal, which is worse than the
    // over-trust it would replace.
    //
    // What IS sound, and is enforced below:
    //   * both rosters are arrays of strings with no repeats;
    //   * every declared name is a member of `profile_domains` — the
    //     declaration and that roster are built from the same descriptors, so
    //     both are canonical wire names and no alias can appear;
    //   * a declared-unscheduled name may not appear verbatim in
    //     `scheduled_domains`. An alias simply does not fire this check, so it
    //     can never refuse a real recording, and it catches a manifest claiming
    //     a domain was unscheduled that its own schedule names.
    //
    // The direction that stays unchecked is a manifest UNDER-declaring, and the
    // one step that closes it is producer-side: the recorder publishing the
    // profile domains it actually scheduled by canonical wire name, alongside
    // the difference it already publishes. Replay would then reconcile exactly,
    // with no registry of its own.
    let unique_names = |rows: &Vec<String>, field: &str| -> Result<BTreeSet<String>, SourceError> {
        let set = rows.iter().cloned().collect::<BTreeSet<_>>();
        if set.len() != rows.len() {
            return Err(SourceError::Open(format!(
                "recorder {field} repeats a domain name ({rows:?})"
            )));
        }
        Ok(set)
    };
    let profile_domains = unique_names(
        &{
            let rows = profile.get("profile_domains").and_then(Value::as_array).ok_or_else(|| {
                SourceError::Open(
                    "recorder production_profile.profile_domains is absent or is not an array"
                        .into(),
                )
            })?;
            rows.iter()
                .enumerate()
                .map(|(index, row)| {
                    row.as_str().map(str::to_string).ok_or_else(|| {
                        SourceError::Open(format!(
                            "recorder production_profile.profile_domains[{index}] is not a string"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        },
        "production_profile.profile_domains",
    )?;
    let scheduled_domains = unique_names(
        &{
            let rows = manifest
                .get("scheduled_domains")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SourceError::Open(
                        "recorder manifest scheduled_domains is absent or is not an array while a \
                         production_profile authority is present"
                            .into(),
                    )
                })?;
            rows.iter()
                .enumerate()
                .map(|(index, row)| {
                    row.as_str().map(str::to_string).ok_or_else(|| {
                        SourceError::Open(format!(
                            "recorder manifest scheduled_domains[{index}] is not a string"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        },
        "manifest scheduled_domains",
    )?;
    let declared_not_scheduled = unique_names(
        &profile_domains_not_scheduled,
        "production_profile.profile_domains_not_scheduled",
    )?;
    if let Some(unknown) = declared_not_scheduled
        .iter()
        .find(|name| !profile_domains.contains(*name))
    {
        return Err(SourceError::Open(format!(
            "recorder production_profile.profile_domains_not_scheduled names {unknown:?}, which is \
             not in production_profile.profile_domains"
        )));
    }
    if let Some(contradicted) = declared_not_scheduled
        .intersection(&scheduled_domains)
        .next()
    {
        return Err(SourceError::Open(format!(
            "recorder production_profile.profile_domains_not_scheduled names {contradicted:?}, \
             which the manifest's own scheduled_domains carries"
        )));
    }

    // Rule 2. A producer that is not production-complete is a correctness
    // statement about the recorder build, not a scope statement about this
    // capture, so no attach mode carves it out.
    if !mandatory_domains_not_production_complete.is_empty() {
        return Err(SourceError::Open(format!(
            "recorder production_profile.mandatory_domains_not_production_complete is nonempty \
             ({mandatory_domains_not_production_complete:?}); those domains' producers are not \
             production-complete in the recording's own recorder build, which no capture mode \
             explains and which the late-attach admission does not cover"
        )));
    }

    // Rule 3. A capture that claims the profile is held to all of it.
    if enabled {
        for (field, rows) in [
            (
                "profile_domains_not_scheduled",
                &profile_domains_not_scheduled,
            ),
            (
                "missing_required_static_artifacts",
                &missing_required_static_artifacts,
            ),
        ] {
            if !rows.is_empty() {
                return Err(SourceError::Open(format!(
                    "recorder production_profile.{field} is nonempty ({rows:?}) while \
                     production_profile.enabled is true; a capture that claims the production \
                     profile must satisfy all of it"
                )));
            }
        }
        return Ok(RecorderProductAdmission {
            authority: ProductionProfileAuthority::Enabled,
            guarantees_not_held: Vec::new(),
            profile_domains_not_scheduled,
            missing_required_static_artifacts,
            static_artifacts_not_content_bound: Vec::new(),
        });
    }

    // Rule 1. Admit, naming every guarantee the capture does not hold.
    let mut guarantees_not_held = vec![ProductionProfileGuarantee::ProductionProfileEnabled];
    if !profile_domains_not_scheduled.is_empty() {
        guarantees_not_held.push(ProductionProfileGuarantee::FullProfileDomainSchedule);
    }
    // `RequiredStaticArtifacts` is NOT decided here. It is the one guarantee
    // replay can verify against the package rather than take on the manifest's
    // word, and it is decided once, at the roster join
    // ([`MetadataRoute::admit_static_artifacts`]), from what the package holds.
    // Two places deciding one guarantee — one from the declaration, one from
    // the package — is what let them disagree (Codex round 8, MEDIUM).
    guarantees_not_held.sort_unstable();
    guarantees_not_held.dedup();
    Ok(RecorderProductAdmission {
        authority: ProductionProfileAuthority::Disabled,
        guarantees_not_held,
        profile_domains_not_scheduled,
        missing_required_static_artifacts,
        static_artifacts_not_content_bound: Vec::new(),
    })
}

fn read_optional_metadata_object_once<F>(
    read_once: F,
    context: &str,
) -> Result<Option<Map<String, Value>>, SourceError>
where
    F: FnOnce() -> Result<Option<Vec<u8>>, SourceError>,
{
    read_once()?
        .map(|bytes| parse_metadata_object(&bytes, context))
        .transpose()
}

fn session_manifest_reference(
    timeline: &Map<String, Value>,
) -> Result<Option<String>, SourceError> {
    let Some(value) = timeline.get("session_manifest") else {
        return Ok(None);
    };
    let reference = value.as_str().ok_or_else(|| {
        SourceError::Open("timeline.json.session_manifest is present but is not a string".into())
    })?;
    if reference.trim().is_empty()
        || reference.starts_with('/')
        || reference.contains(char::from(92))
        || reference
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || snapshot::zip::validate_path(reference).is_err()
    {
        return Err(SourceError::Open(format!(
            "timeline.json.session_manifest is not a valid game-relative forward-slash path: {reference:?}"
        )));
    }
    Ok(Some(reference.to_string()))
}

fn recorder_manifest_binding_value(
    root: &Map<String, Value>,
    reference: &str,
    product_admission: RecorderProductAdmission,
) -> Result<RecorderSessionBinding, SourceError> {
    let campaign_identity = recorder_campaign_identity(&root, reference)?;
    let modset = root
        .get("modset")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset is missing or not an object"
            ))
        })?;

    match modset.get("resolved") {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => {
            let attributed = modset
                .get("errors")
                .and_then(Value::as_array)
                .map(|errors| {
                    errors
                        .iter()
                        .map(|error| {
                            error
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| format!("<non-string {error}>"))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec!["<missing or malformed errors>".into()]);
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} modset is unresolved; attributed errors: {attributed:?}"
            )));
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} modset.resolved is missing or not boolean"
            )))
        }
    }

    match modset.get("errors") {
        Some(Value::Array(errors)) if errors.is_empty() => {}
        Some(Value::Array(errors)) => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} resolved modset must have errors == []; found {errors:?}"
            )))
        }
        _ => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} resolved modset errors is missing or not an array"
            )))
        }
    }

    let dlc = modset
        .get("dlc")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc is missing or not an object"
            ))
        })?;
    let enabled = dlc
        .get("enabled")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc.enabled is missing or not an array"
            ))
        })?;
    let enabled_paths: Vec<&str> = enabled
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder manifest {reference:?} modset.dlc.enabled[{index}] is not a string"
                ))
            })
        })
        .collect::<Result<_, _>>()?;

    let display_names = modset
        .get("dlc_display_names")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.dlc_display_names is missing or not an object"
            ))
        })?;
    let mut active_dlcs = Vec::with_capacity(enabled_paths.len());
    for path in enabled_paths {
        let display = display_names
            .get(path)
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder metadata binding check: enabled DLC path {path:?} has no exactly-mapped nonblank display name"
                ))
            })?;
        // A HOLE IS NOT A NAME. The seal removes any value it could not make
        // portable and leaves a reserved stand-in in its place, accounted in
        // the container's `portability_exclusions` ledger. A DLC display name
        // is AUTHORED content, so one can carry an absolute path and be
        // removed; reading the stand-in as the name would publish a recording
        // whose active-DLC list is fiction.
        if display == PORTABILITY_EXCLUDED_VALUE {
            return Err(SourceError::Open(format!(
                "recorder metadata binding check: enabled DLC path {path:?} has no display \
                 name in this container -- the seal excluded it as non-portable, and the \
                 portability_exclusions ledger names it"
            )));
        }
        active_dlcs.push(display.to_string());
    }
    let settings_binding = modset.get("settings_binding").ok_or_else(|| {
        SourceError::Open(format!(
            "recorder manifest {reference:?} modset.settings_binding is missing"
        ))
    })?;
    let localisation_language = settings_binding
        .get("language")
        .and_then(Value::as_str)
        .filter(|language| {
            !language.trim().is_empty() && *language != PORTABILITY_EXCLUDED_VALUE
        })
        .ok_or_else(|| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.settings_binding.language is missing, \
                 blank, or excluded as non-portable"
            ))
        })?
        .to_string();
    let display_variant: RecorderDisplayVariant =
        serde_json::from_value(recorder_display_variant_value(settings_binding).ok_or_else(
            || {
                SourceError::Open(format!(
                    "recorder manifest {reference:?} modset.settings_binding.display_variant \
                     is missing"
                ))
            },
        )?)
        .map_err(|error| {
            SourceError::Open(format!(
                "recorder manifest {reference:?} modset.display_variant has invalid shape: {error}"
            ))
        })?;
    validate_display_variant(&display_variant, reference)?;

    Ok(RecorderSessionBinding {
        active_dlcs,
        localisation_language,
        display_variant,
        campaign_identity,
        product_admission,
    })
}

fn recorder_campaign_identity(
    root: &Map<String, Value>,
    reference: &str,
) -> Result<Option<RecordedCampaignIdentity>, SourceError> {
    const OBJECT_KEYS: &[&str] = &[
        "schema",
        "state",
        "campaign_id",
        "game_unique_id",
        "game_unique_seed",
        "source",
        "source_build_profile",
        "observed_game_hour_raw",
        "stream_header_sha256",
        "resolution_note",
        "witnesses",
    ];
    const WITNESS_KEYS: &[&str] = &[
        "start_date_raw",
        "session",
        "scenario",
        "initial_human_country_tags",
        "modset_identity",
    ];
    let Some(value) = root.get("campaign_identity") else {
        return Ok(None);
    };
    let fail = |message: String| {
        SourceError::Open(format!(
            "recorder manifest {reference:?} campaign_identity {message}"
        ))
    };
    let object = value
        .as_object()
        .ok_or_else(|| fail("is not an object".into()))?;
    if object.len() != OBJECT_KEYS.len() || OBJECT_KEYS.iter().any(|key| !object.contains_key(*key))
    {
        return Err(fail("does not have the exact recorder-written key set".into()));
    }
    if object.get("schema").and_then(Value::as_str) != Some("hoi4.campaign_identity.v1") {
        return Err(fail("has an unsupported schema".into()));
    }
    let witnesses = object
        .get("witnesses")
        .and_then(Value::as_object)
        .ok_or_else(|| fail("witnesses is not an object".into()))?;
    if witnesses.len() != WITNESS_KEYS.len()
        || WITNESS_KEYS.iter().any(|key| !witnesses.contains_key(*key))
    {
        return Err(fail("witnesses does not have the exact recorder-written key set".into()));
    }
    for key in ["start_date_raw", "session"] {
        let field = &witnesses[key];
        if !field.is_null()
            && field
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .is_none()
        {
            return Err(fail(format!("witness {key} is not null or u32")));
        }
    }
    for key in ["scenario", "modset_identity"] {
        let field = &witnesses[key];
        if !field.is_null() && field.as_str().is_none() {
            return Err(fail(format!("witness {key} is not null or string")));
        }
    }
    let roster = &witnesses["initial_human_country_tags"];
    if !roster.is_null()
        && roster
            .as_array()
            .map_or(true, |rows| rows.iter().any(|row| row.as_str().is_none()))
    {
        return Err(fail(
            "witness initial_human_country_tags is not null or a string array".into(),
        ));
    }
    let recorded_witnesses = RecordedCampaignIdentityWitnesses {
        start_date_raw: witnesses["start_date_raw"]
            .as_u64()
            .map(|value| value as u32),
        session: witnesses["session"].as_u64().map(|value| value as u32),
        scenario: witnesses["scenario"].as_str().map(str::to_string),
        initial_human_country_tags: roster.as_array().map(|rows| {
            rows.iter()
                .map(|row| row.as_str().expect("witness roster validated").to_string())
                .collect()
        }),
        modset_identity: witnesses["modset_identity"].as_str().map(str::to_string),
    };

    let state = object
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("state is not a string".into()))?;
    let nullable_string = |key: &str| -> Result<Option<String>, SourceError> {
        let value = &object[key];
        if value.is_null() {
            Ok(None)
        } else {
            value
                .as_str()
                .map(|value| Some(value.to_string()))
                .ok_or_else(|| fail(format!("{key} is not null or string")))
        }
    };
    let campaign_id = nullable_string("campaign_id")?;
    let game_unique_id = nullable_string("game_unique_id")?;
    let game_unique_seed = nullable_string("game_unique_seed")?;
    let source = nullable_string("source")?;
    let source_build_profile = nullable_string("source_build_profile")?;
    let stream_header_sha256 = nullable_string("stream_header_sha256")?;
    let resolution_note = nullable_string("resolution_note")?;
    if source_build_profile
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
    {
        return Err(fail("source_build_profile violates its closed grammar".into()));
    }
    let observed_game_hour_raw = if object["observed_game_hour_raw"].is_null() {
        None
    } else {
        let raw = object["observed_game_hour_raw"]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| fail("observed_game_hour_raw is not null or i32".into()))?;
        // The extractor applies this same window when it stamps the hour; it
        // is shared rather than re-typed so the two cannot drift.
        if !portable_admission::raw_date_in_range(raw) {
            return Err(fail("observed_game_hour_raw is outside the compact calendar".into()));
        }
        Some(raw)
    };
    let lower_hex64 = |value: &str| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };

    match state {
        "known" => {
            let campaign_id = campaign_id
                .as_deref()
                .filter(|value| lower_hex64(value))
                .ok_or_else(|| fail("known campaign_id is not lowercase SHA-256".into()))?;
            let uuid = game_unique_id
                .as_deref()
                .filter(|value| canonical_campaign_uuid(value))
                .ok_or_else(|| fail("known game_unique_id is not a canonical lowercase UUID".into()))?;
            let seed = game_unique_seed
                .as_deref()
                .and_then(|value| value.parse::<i32>().ok().map(|parsed| (value, parsed)))
                .filter(|(value, parsed)| *value == parsed.to_string())
                .map(|(value, _)| value)
                .ok_or_else(|| fail("known game_unique_seed is not canonical signed i32".into()))?;
            let mut preimage = b"hoi4-campaign-id-v1\0".to_vec();
            preimage.extend_from_slice(uuid.as_bytes());
            preimage.push(0);
            preimage.extend_from_slice(seed.as_bytes());
            let recomputed = snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&preimage));
            if recomputed != campaign_id {
                return Err(fail("known campaign digest does not recompute".into()));
            }
            if source.as_deref() != Some("live_memory")
                || source_build_profile.is_none()
                || observed_game_hour_raw.is_none()
                || stream_header_sha256
                    .as_deref()
                    .map_or(true, |value| !lower_hex64(value))
                || resolution_note.is_some()
            {
                return Err(fail("known state fields are incoherent".into()));
            }
        }
        "unavailable" | "unbound" | "identity_component_conflict" => {
            if campaign_id.is_some()
                || game_unique_id.is_some()
                || game_unique_seed.is_some()
                || source.is_some()
                || observed_game_hour_raw.is_some()
                || stream_header_sha256.is_some()
                || resolution_note.as_deref().map_or(true, str::is_empty)
            {
                return Err(fail("non-known state contains positive identity authority".into()));
            }
        }
        _ => return Err(fail(format!("has unknown state {state:?}"))),
    }
    Ok(Some(RecordedCampaignIdentity {
        state: state.to_string(),
        campaign_id,
        game_unique_id,
        game_unique_seed,
        source,
        source_build_profile,
        observed_game_hour_raw,
        stream_header_sha256,
        resolution_note,
        witnesses: recorded_witnesses,
    }))
}

fn canonical_campaign_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
        })
}

fn validate_display_variant(
    display: &RecorderDisplayVariant,
    reference: &str,
) -> Result<(), SourceError> {
    let scale = f32::from_bits(display.gui_scale_f32_bits);
    let expected_mode = match (display.fullscreen, display.borderless) {
        (true, false) => "fullscreen",
        (false, true) => "borderless_fullscreen",
        (false, false) => "windowed",
        (true, true) => {
            return Err(SourceError::Open(format!(
                "recorder manifest {reference:?} display_variant flags are ambiguous"
            )))
        }
    };
    let sha_ok = display.settings_file_sha256.len() == 64
        && display
            .settings_file_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if display.physical_width == 0
        || display.physical_height == 0
        || display.min_gui_width == 0
        || display.min_gui_height == 0
        || display.refresh_rate_hz == 0
        || !scale.is_finite()
        || scale <= 0.0
        || display.display_mode != expected_mode
        || !sha_ok
    {
        return Err(SourceError::Open(format!(
            "recorder manifest {reference:?} modset.display_variant violates the exact launch-settings contract"
        )));
    }
    Ok(())
}

/// Precise Unsupported error for a recording whose snapshot index is empty.
/// When positive compact-stream evidence is present the error is the TYPED
/// [`SourceError::UnsupportedCompactStreamOnly`] subclass (R18-5) — this
/// function is that variant's sole producer — so consumers classify the
/// compact-stream-only flavor structurally rather than by message text.
fn empty_index_error(store: &Store, timeline: Option<&Value>, game_id: &str) -> SourceError {
    let contract = "playback requires certified replay_decoder hour-index/checkpoint \
                    roles, legacy per-hour snapshot entries, or an explicitly supplied \
                    certified portable pre-folder; ordinary product open does not guess \
                    compact record layouts";
    match compact_stream_evidence(store, timeline) {
        Some(evidence) => SourceError::UnsupportedCompactStreamOnly(format!(
            "recording {game_id:?} is a compact-stream-only production recording \
             (empty snapshot index; {evidence}): it carries only the raw compact \
             domain stream, and {contract}"
        )),
        None => SourceError::Unsupported(format!(
            "recording {game_id:?} has an empty snapshot index (\"snapshots\": []) \
             and no per-hour snapshots; {contract}"
        )),
    }
}

/// Evidence that a recording is the production compact-stream flavor: a
/// container manifest entry with role `collector_compact_stream`, or a
/// `timeline.json` declaring format `hoi4_compact_stream_timeline_v1`.
fn compact_stream_evidence(store: &Store, timeline: Option<&Value>) -> Option<String> {
    if let Some(container) = store.container() {
        let compact = container.entries_by_role(COMPACT_STREAM_ROLE);
        if let Some(entry) = compact.first() {
            return Some(format!(
                "manifest entry {:?} has role {COMPACT_STREAM_ROLE:?}",
                entry.path
            ));
        }
        return timeline.and_then(timeline_compact_evidence);
    }
    timeline.and_then(timeline_compact_evidence)
}

fn timeline_compact_evidence(timeline: &Value) -> Option<String> {
    let format = timeline.get("format").and_then(Value::as_str);
    let role = timeline.get("raw_stream_role").and_then(Value::as_str);
    if format == Some(COMPACT_STREAM_TIMELINE_FORMAT) || role == Some(COMPACT_STREAM_ROLE) {
        Some(format!(
            "timeline.json declares format {:?} / raw_stream_role {:?}",
            format.unwrap_or("<missing>"),
            role.unwrap_or("<missing>")
        ))
    } else {
        None
    }
}

/// Backend 1: a snapshot-flavor container or directory store, read through
/// the verified `snapshot` crate. Never leaks `snapshot` types.
///
/// F2 audit note (justified O(recorded-hours) floor): `hours` (8 B/hour) and
/// `files_by_hour` (one recorded file name per hour) are the random-access
/// index of the recording — built once at open, never growing with scrub
/// count. They are required to serve `frame(hour)` without re-reading the
/// store index and are the documented per-open floor of the source's working
/// set (~210 KB + file-name strings for a 3-year hourly recording).
/// `warnings` is bounded by the number of malformed index entries.
static PREFOLD_SPOOL_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const PREFOLD_SPOOL_MARKER: &str = ".hoi4-prefold-owner";

/// The refusal text when `CertifiedReplayCache` will not admit the packaged
/// artifacts at all.
///
/// When the container ALSO refused the entry, its complaint is carried into the
/// sentence. The reordering in `open_packaged_replay_cache` means the
/// container's refusal is no longer what ends the open, and a reader that saw
/// only "admit packaged replay_decoder binary artifacts: …" would have lost the
/// fact that the container's own account of the entry disagreed too — which is
/// usually the more diagnostic of the two.
fn admission_refusal(
    error: &snapshot::ReplayCacheError,
    copy_outcome: &Result<snapshot::SpoolEntryReceipt, snapshot::ContainerError>,
) -> String {
    match copy_outcome {
        Ok(_) => format!("admit packaged replay_decoder binary artifacts: {error}"),
        Err(copy_error) => format!(
            "admit packaged replay_decoder binary artifacts: {error} (the container also \
             refused the entry: {copy_error})"
        ),
    }
}

/// RAII ownership for one private bounded pre-fold workspace. Cleanup is
/// attempted only when the exact canonical directory and unique-per-open
/// marker still match; an identity discrepancy leaves evidence in place rather
/// than recursively deleting an unknown target.
/// Open this process's own checkpoint-cache spool for bounded record reads.
///
/// The pathname is the one just created inside this process's private
/// [`PrefoldSpoolOwner`] root and written by the entry copy in
/// `open_packaged_replay_cache`. Nothing outside that root is ever named here,
/// and the file is opened read-only: the cache reader seeks in it, it never
/// writes to it.
///
/// It is NOT published by the container's transactional spool any more (task
/// #192): the container's verdict on the entry can no longer decide whether the
/// bytes are written, because the sealed H4IX judges them record by record
/// afterwards. So the bytes in this file are unjudged at the moment it is
/// opened. The root's privacy is what makes that safe — nothing outside this
/// call can see it, it is removed with the source, and nothing leaves
/// `CertifiedReplayCache` that has not matched a digest the seal published.
fn open_private_prefold_spool_file(spool_path: &Path) -> Result<fs::File, SourceError> {
    let spool_file = fs::OpenOptions::new()
        .read(true)
        .open(spool_path)
        .map_err(|error| {
            SourceError::Open(format!(
                "open private checkpoint cache spool {}: {error}",
                spool_path.display()
            ))
        })?;
    Ok(spool_file)
}

struct PrefoldSpoolOwner {
    root: PathBuf,
    canonical_root: PathBuf,
    snapshot_dir: PathBuf,
    marker_value: String,
}

impl PrefoldSpoolOwner {
    fn create(base: &Path) -> Result<Self, SourceError> {
        let meta = fs::symlink_metadata(base).map_err(|error| {
            SourceError::Open(format!(
                "inspect prefold spool base {}: {error}",
                base.display()
            ))
        })?;
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return Err(SourceError::Open(format!(
                "prefold spool base is not a regular non-symlink directory: {}",
                base.display()
            )));
        }
        let canonical_base = fs::canonicalize(base).map_err(|error| {
            SourceError::Open(format!(
                "canonicalize prefold spool base {}: {error}",
                base.display()
            ))
        })?;
        let sequence = PREFOLD_SPOOL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| SourceError::Open(format!("prefold spool clock: {error}")))?
            .as_nanos();
        let marker_value = format!("{}-{now:032x}-{sequence:016x}", std::process::id());

        for attempt in 0..64u64 {
            let root = canonical_base.join(format!(
                ".hoi4-prefold-{}-{now:032x}-{:016x}",
                std::process::id(),
                sequence.wrapping_add(attempt)
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    let canonical_root = match fs::canonicalize(&root) {
                        Ok(path) if path.parent() == Some(canonical_base.as_path()) => path,
                        Ok(_) => {
                            let _ = fs::remove_dir(&root);
                            return Err(SourceError::Open(
                                "private prefold spool escaped its canonical base".to_string(),
                            ));
                        }
                        Err(error) => {
                            let _ = fs::remove_dir(&root);
                            return Err(SourceError::Open(format!(
                                "canonicalize private prefold spool {}: {error}",
                                root.display()
                            )));
                        }
                    };
                    let setup = (|| -> Result<PathBuf, SourceError> {
                        let marker_path = canonical_root.join(PREFOLD_SPOOL_MARKER);
                        let mut marker = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&marker_path)
                            .map_err(|error| {
                                SourceError::Open(format!(
                                    "create prefold ownership marker {}: {error}",
                                    marker_path.display()
                                ))
                            })?;
                        marker.write_all(marker_value.as_bytes()).map_err(|error| {
                            SourceError::Open(format!(
                                "write prefold ownership marker {}: {error}",
                                marker_path.display()
                            ))
                        })?;
                        marker.sync_all().map_err(|error| {
                            SourceError::Open(format!(
                                "sync prefold ownership marker {}: {error}",
                                marker_path.display()
                            ))
                        })?;
                        let snapshot_dir = canonical_root.join("snapshots");
                        fs::create_dir(&snapshot_dir).map_err(|error| {
                            SourceError::Open(format!(
                                "create private snapshot spool {}: {error}",
                                snapshot_dir.display()
                            ))
                        })?;
                        Ok(snapshot_dir)
                    })();
                    match setup {
                        Ok(snapshot_dir) => {
                            return Ok(Self {
                                root: canonical_root.clone(),
                                canonical_root,
                                snapshot_dir,
                                marker_value,
                            });
                        }
                        Err(error) => {
                            if fs::canonicalize(&root).ok().as_deref()
                                == Some(canonical_root.as_path())
                            {
                                let _ = fs::remove_dir_all(&root);
                            }
                            return Err(error);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(SourceError::Open(format!(
                        "create private prefold spool {}: {error}",
                        root.display()
                    )))
                }
            }
        }
        Err(SourceError::Open(
            "could not allocate a unique private prefold spool directory".to_string(),
        ))
    }

    fn root(&self) -> &Path {
        &self.root
    }
    fn snapshot_dir(&self) -> &Path {
        &self.snapshot_dir
    }

    fn identity_intact(&self) -> bool {
        let Ok(meta) = fs::symlink_metadata(&self.root) else {
            return false;
        };
        if !meta.file_type().is_dir() || meta.file_type().is_symlink() {
            return false;
        }
        if fs::canonicalize(&self.root).ok().as_deref() != Some(self.canonical_root.as_path()) {
            return false;
        }
        let Ok(snapshot_meta) = fs::symlink_metadata(&self.snapshot_dir) else {
            return false;
        };
        if !snapshot_meta.file_type().is_dir() || snapshot_meta.file_type().is_symlink() {
            return false;
        }
        if fs::canonicalize(&self.snapshot_dir).ok().as_deref() != Some(self.snapshot_dir.as_path())
        {
            return false;
        }
        let marker = self.root.join(PREFOLD_SPOOL_MARKER);
        let Ok(marker_meta) = fs::symlink_metadata(&marker) else {
            return false;
        };
        if !marker_meta.file_type().is_file() || marker_meta.file_type().is_symlink() {
            return false;
        }
        if fs::read_to_string(marker).ok().as_deref() != Some(self.marker_value.as_str()) {
            return false;
        }
        let Ok(root_entries) = fs::read_dir(&self.root) else {
            return false;
        };
        for entry in root_entries {
            let Ok(entry) = entry else {
                return false;
            };
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return false;
            };
            // The private root holds the marker, the snapshot directory, and the
            // two large artifacts this process spools into it: the raw H4CR for
            // the folding seam and the certified checkpoint cache for the
            // packaged one. Anything else means someone else has been writing
            // here, and the owner disowns the directory rather than deleting
            // whatever it now contains.
            let spooled_artifact = name == "source.h4cr" || name == "checkpoints.h4cp";
            if name != PREFOLD_SPOOL_MARKER && !spooled_artifact && name != "snapshots" {
                return false;
            }
            if spooled_artifact {
                let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                    return false;
                };
                if !meta.file_type().is_file() || meta.file_type().is_symlink() {
                    return false;
                }
                let Ok(canonical) = fs::canonicalize(entry.path()) else {
                    return false;
                };
                if canonical.parent() != Some(self.root.as_path()) {
                    return false;
                }
            }
        }
        let Ok(snapshot_entries) = fs::read_dir(&self.snapshot_dir) else {
            return false;
        };
        for entry in snapshot_entries {
            let Ok(entry) = entry else {
                return false;
            };
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                return false;
            };
            if !meta.file_type().is_file() || meta.file_type().is_symlink() {
                return false;
            }
            let Ok(canonical) = fs::canonicalize(entry.path()) else {
                return false;
            };
            if canonical.parent() != Some(self.snapshot_dir.as_path()) {
                return false;
            }
        }
        true
    }

    fn require_identity(&self) -> Result<(), SourceError> {
        if self.identity_intact() {
            Ok(())
        } else {
            Err(SourceError::Open(
                "private prefold spool identity changed; refusing read and automatic cleanup"
                    .to_string(),
            ))
        }
    }

    fn remove_authenticated_source(&self) -> Result<(), SourceError> {
        self.require_identity()?;
        let path = self.root.join("source.h4cr");
        let meta = fs::symlink_metadata(&path).map_err(|error| {
            SourceError::Open(format!(
                "inspect authenticated source spool {}: {error}",
                path.display()
            ))
        })?;
        let canonical = fs::canonicalize(&path).map_err(|error| {
            SourceError::Open(format!(
                "canonicalize authenticated source spool {}: {error}",
                path.display()
            ))
        })?;
        if !meta.file_type().is_file()
            || meta.file_type().is_symlink()
            || canonical.parent() != Some(self.root.as_path())
        {
            return Err(SourceError::Open(
                "authenticated source spool identity changed before cleanup".to_string(),
            ));
        }
        fs::remove_file(&path).map_err(|error| {
            SourceError::Open(format!(
                "remove authenticated source spool {}: {error}",
                path.display()
            ))
        })
    }
}

impl Drop for PrefoldSpoolOwner {
    fn drop(&mut self) {
        if self.identity_intact() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn read_spooled_hour(
    owner: &PrefoldSpoolOwner,
    snapshot_dir: &Path,
    spool: &snapshot::PrefoldedSpool,
    entry: &snapshot::PrefoldedSpoolEntry,
) -> Result<snapshot::Snapshot, SourceError> {
    owner.require_identity()?;
    if fs::canonicalize(snapshot_dir).ok().as_deref() != Some(owner.snapshot_dir()) {
        return Err(SourceError::Open(
            "private snapshot spool identity changed".to_string(),
        ));
    }
    let path = snapshot_dir.join(entry.file_name());
    let canonical = fs::canonicalize(&path).map_err(|error| {
        SourceError::Open(format!(
            "canonicalize spooled snapshot {}: {error}",
            path.display()
        ))
    })?;
    if canonical.parent() != Some(snapshot_dir) {
        return Err(SourceError::Open(format!(
            "spooled snapshot escaped private directory: {}",
            path.display()
        )));
    }
    let path_meta = fs::symlink_metadata(&path).map_err(|error| {
        SourceError::Open(format!(
            "inspect spooled snapshot {}: {error}",
            path.display()
        ))
    })?;
    if !path_meta.file_type().is_file()
        || path_meta.file_type().is_symlink()
        || path_meta.len() != entry.byte_len()
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot metadata changed for hour {}",
            entry.hour_index()
        )));
    }
    let file = fs::File::open(&canonical).map_err(|error| {
        SourceError::Open(format!(
            "open spooled snapshot {}: {error}",
            canonical.display()
        ))
    })?;
    if file
        .metadata()
        .map_err(|error| SourceError::Open(error.to_string()))?
        .len()
        != entry.byte_len()
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot length changed while opening hour {}",
            entry.hour_index()
        )));
    }
    let mut bytes = Vec::new();
    file.take(entry.byte_len().saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            SourceError::Open(format!(
                "read spooled snapshot for hour {}: {error}",
                entry.hour_index()
            ))
        })?;
    if bytes.len() as u64 != entry.byte_len() {
        return Err(SourceError::Open(format!(
            "spooled snapshot byte count changed for hour {}",
            entry.hour_index()
        )));
    }
    let actual_sha = snapshot::sha256::to_hex(&snapshot::sha256::Sha256::digest(&bytes));
    if actual_sha != entry.sha256() {
        return Err(SourceError::Open(format!(
            "spooled snapshot SHA-256 changed for hour {}",
            entry.hour_index()
        )));
    }
    let strict = snapshot::parse_json_strict(&bytes).map_err(|error| {
        SourceError::Open(format!(
            "spooled hour {} is not strict snapshot JSON: {error}",
            entry.hour_index()
        ))
    })?;
    let snapshot: snapshot::Snapshot = serde_json::from_value(strict).map_err(|error| {
        SourceError::Open(format!(
            "spooled hour {} is not canonical snapshot JSON: {error}",
            entry.hour_index()
        ))
    })?;
    if snapshot.hour_index != Some(entry.hour_index())
        || snapshot.game_id.as_deref() != Some(spool.game_id())
        || snapshot.schema_version.as_ref() != Some(spool.schema_version())
        || snapshot.date.as_deref() != Some(entry.date())
    {
        return Err(SourceError::Open(format!(
            "spooled snapshot identity disagrees with certified manifest at hour {}",
            entry.hour_index()
        )));
    }
    Ok(snapshot)
}

struct StoreSource {
    store: Store,
    game_id: String,
    hours: Vec<i64>,
    files_by_hour: BTreeMap<i64, String>,
    /// Authenticated index dates parallel to `files_by_hour`. A missing date
    /// stays missing; reading navigation metadata never opens a frame body.
    dates_by_hour: BTreeMap<i64, Option<String>>,
    /// The index's own declared coverage statement, carried verbatim.
    declared_coverage: DeclaredCoverage,
    warnings: Vec<String>,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
}

struct IndexedReplaySource {
    cache: snapshot::CertifiedReplayCache,
    /// Private spool the cache reads from, when this recording's cache was too
    /// large to hold. `None` means the cache is resident. Dropped with the
    /// source, which removes the directory.
    #[allow(dead_code)]
    cache_spool: Option<PrefoldSpoolOwner>,
    hours: Vec<i64>,
    warnings: Vec<String>,
    store: Store,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    prefold_receipt: snapshot::PrefoldSourceReceipt,
}

struct SpooledPrefoldedSource {
    spool: snapshot::PrefoldedSpool,
    snapshot_dir: PathBuf,
    owner: PrefoldSpoolOwner,
    warnings: Vec<String>,
    store: Store,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    prefold_receipt: snapshot::PrefoldSourceReceipt,
}

/// Neutral, immutable already-folded backend. The portable contract owns the
/// index/entry identity checks; this adapter only projects each typed snapshot
/// into replay_core's existing raw-frame boundary.
struct PrefoldedSource {
    replay: snapshot::PrefoldedReplay,
    warnings: Vec<String>,
    store: Option<Store>,
    recorder_binding: Option<RecorderSessionBinding>,
    design_catalog: Option<Arc<crate::design_catalog::PortableDesignCatalog>>,
    decision_catalog: Option<PackagedDecisionCatalog>,
    static_artifacts: BTreeMap<String, PackagedStaticArtifact>,
    prefold_receipt: Option<snapshot::PrefoldSourceReceipt>,
}

impl ReplaySource for IndexedReplaySource {
    fn game_id(&self) -> &str {
        self.cache.index().game_id()
    }

    /// This is the one source whose `frame` is worth abandoning: it decodes a
    /// checkpoint and applies every delta to the wanted hour, which is seconds
    /// of work at a 744-hour interval. The switch goes to the cache reader,
    /// which consults it at every delta-record boundary and keeps the position
    /// it reached.
    fn set_scrub_interrupt(&self, interrupt: Option<Arc<dyn Fn() -> bool + Send + Sync>>) {
        self.cache.set_scrub_interrupt(interrupt);
    }

    fn hours(&self) -> &[i64] {
        &self.hours
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .container()
            .and_then(|container| container.content_manifest())
    }

    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.recorder_binding
            .as_ref()
            .map(|binding| &binding.product_admission)
    }



    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }

    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        Some(&self.prefold_receipt)
    }

    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        self.cache
            .index()
            .entry(hour_index)
            .map(|entry| Some(entry.date().to_string()))
            .ok_or_else(|| {
                SourceError::Open(format!(
                    "no indexed replay_decoder hour {hour_index} in game {:?}",
                    self.game_id()
                ))
            })
    }

    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let (json, receipt) = self
            .cache
            .reconstruct_json(hour_index)
            .map_err(|error| {
                SourceError::Open(format!(
                    "reconstruct replay_decoder hour {hour_index}: {error}"
                ))
            })?;
        if receipt.applied_deltas > receipt.configured_bound {
            return Err(SourceError::Open(format!(
                "replay_decoder hour {hour_index} exceeded its certified seek bound"
            )));
        }
        let mut snapshot: snapshot::Snapshot = snapshot::parse_json_strict(&json)
            .and_then(serde_json::from_value)
            .map_err(|error| {
                SourceError::Open(format!(
                    "replay_decoder hour {hour_index} is not canonical snapshot JSON: {error}"
                ))
            })?;
        if snapshot.hour_index != Some(hour_index)
            || snapshot.game_id.as_deref() != Some(self.game_id())
        {
            return Err(SourceError::Open(format!(
                "replay_decoder reconstructed body identity disagrees at hour {hour_index}"
            )));
        }
        let mut rest = std::mem::take(&mut snapshot.rest);
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snapshot.date,
            save_version: snapshot.save_version,
            schema_version: snapshot.schema_version,
            rest,
        })
    }
}

impl ReplaySource for SpooledPrefoldedSource {
    fn game_id(&self) -> &str {
        self.spool.game_id()
    }
    fn hours(&self) -> &[i64] {
        self.spool.hours()
    }
    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .container()
            .and_then(|container| container.content_manifest())
    }

    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.recorder_binding
            .as_ref()
            .map(|binding| &binding.product_admission)
    }



    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }

    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        Some(&self.prefold_receipt)
    }

    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        let entry = self.spool.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no spooled prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        Ok(Some(entry.date().to_string()))
    }

    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let entry = self.spool.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no spooled prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        let snapshot = read_spooled_hour(&self.owner, &self.snapshot_dir, &self.spool, entry)?;
        // Admission proved container identity for the FIRST spooled hour only,
        // because this route is deliberately lazy and never materializes the
        // whole spool. Re-prove it for every hour that is actually read, so a
        // later hour whose provenance disagrees with the container manifest is
        // refused at the frame it is served from rather than never.
        if let Some(container) = self.store.container() {
            validate_snapshot_container_identity(container, &snapshot)?;
        }
        let mut rest = snapshot.rest;
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snapshot.date,
            save_version: snapshot.save_version,
            schema_version: snapshot.schema_version,
            rest,
        })
    }
}

impl ReplaySource for PrefoldedSource {
    fn game_id(&self) -> &str {
        self.replay.index().game_id()
    }

    fn hours(&self) -> &[i64] {
        self.replay.index().hours()
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .as_ref()
            .and_then(Store::container)
            .and_then(|container| container.content_manifest())
    }

    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.recorder_binding
            .as_ref()
            .map(|binding| &binding.product_admission)
    }



    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }

    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }

    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }

    fn prefold_receipt(&self) -> Option<&snapshot::PrefoldSourceReceipt> {
        self.prefold_receipt.as_ref()
    }

    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        let entry = self.replay.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        Ok(entry.snapshot().date.clone())
    }

    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let entry = self.replay.entry(hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no prefolded snapshot for hour_index {hour_index} in game {:?}",
                self.game_id()
            ))
        })?;
        let snapshot = entry.snapshot();
        let mut rest = snapshot.rest.clone();
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snapshot.date.clone(),
            save_version: snapshot.save_version.clone(),
            schema_version: snapshot.schema_version.clone(),
            rest,
        })
    }
}

impl ReplaySource for StoreSource {
    fn game_id(&self) -> &str {
        &self.game_id
    }
    fn hours(&self) -> &[i64] {
        &self.hours
    }
    fn warnings(&self) -> &[String] {
        &self.warnings
    }
    fn declared_coverage(&self) -> Option<&DeclaredCoverage> {
        (!self.declared_coverage.is_empty()).then_some(&self.declared_coverage)
    }
    fn content_manifest(&self) -> Option<&portable_content_manifest::ContentManifest> {
        self.store
            .container()
            .and_then(|container| container.content_manifest())
    }
    fn campaign_identity(&self) -> Option<&RecordedCampaignIdentity> {
        self.recorder_binding
            .as_ref()
            .and_then(|binding| binding.campaign_identity.as_ref())
    }

    fn product_admission(&self) -> Option<&RecorderProductAdmission> {
        self.recorder_binding
            .as_ref()
            .map(|binding| &binding.product_admission)
    }

    fn design_catalog(&self) -> Option<&Arc<crate::design_catalog::PortableDesignCatalog>> {
        self.design_catalog.as_ref()
    }
    fn decision_catalog(&self) -> Option<&PackagedDecisionCatalog> {
        self.decision_catalog.as_ref()
    }
    fn static_artifact(&self, name: &str) -> Option<&PackagedStaticArtifact> {
        self.static_artifacts.get(name)
    }
    fn date_at(&self, hour_index: i64) -> Result<Option<String>, SourceError> {
        self.dates_by_hour.get(&hour_index).cloned().ok_or_else(|| {
            SourceError::Open(format!(
                "no snapshot for hour_index {hour_index} in game {:?}",
                self.game_id
            ))
        })
    }
    fn frame(&self, hour_index: i64) -> Result<RawFrame, SourceError> {
        let file = self.files_by_hour.get(&hour_index).ok_or_else(|| {
            SourceError::Open(format!(
                "no snapshot for hour_index {hour_index} in game {:?}",
                self.game_id
            ))
        })?;
        let snap = self
            .store
            .load_file(&self.game_id, file)
            .map_err(|e| source_error_from_load(&e, format!("load snapshot {file:?}: {e}")))?;
        // Join the COMPLETE snapshot header against the index, not just the
        // hour. The spooled sibling refuses a mismatched hour/game/schema/date
        // before it hands a frame back; without the same join here `date_at`
        // (index) and `frame()` (body) can answer differently for one hour and
        // the navigation UI and the frame disagree with no refusal anywhere.
        //
        // The index stays the authority. A header field the body does not carry
        // stays absent (a v1 recording need not repeat it); a field the body
        // DOES carry and that contradicts the index is refused.
        if let Some(body_hour) = snap.hour_index {
            if body_hour != hour_index {
                return Err(SourceError::Open(format!(
                    "snapshot {file:?} body hour_index {body_hour} != index hour_index \
                     {hour_index} (index is the authority; refusing inconsistent recording)"
                )));
            }
        }
        if let Some(body_game) = snap.game_id.as_deref() {
            if body_game != self.game_id {
                return Err(SourceError::Open(format!(
                    "snapshot {file:?} body game_id {body_game:?} != selected game {:?} \
                     (index is the authority; refusing inconsistent recording)",
                    self.game_id
                )));
            }
        }
        if let (Some(index_date), Some(body_date)) = (
            self.dates_by_hour.get(&hour_index).and_then(Option::as_deref),
            snap.date.as_deref(),
        ) {
            if index_date != body_date {
                return Err(SourceError::Open(format!(
                    "snapshot {file:?} body date {body_date:?} != index date {index_date:?} at \
                     hour_index {hour_index} (index is the authority; refusing inconsistent \
                     recording)"
                )));
            }
        }
        let mut rest = snap.rest;
        if let Some(binding) = &self.recorder_binding {
            validate_existing_recorder_metadata(&rest, binding)?;
            let session = rest
                .entry("session".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
            bind_or_match(
                session,
                "active_dlcs",
                serde_json::json!(binding.active_dlcs),
            )?;
            bind_or_match(
                session,
                "localisation_language",
                serde_json::json!(binding.localisation_language),
            )?;
            bind_or_match(
                session,
                "display_variant",
                serde_json::json!(binding.display_variant),
            )?;
        }
        Ok(RawFrame {
            hour_index,
            date: snap.date,
            save_version: snap.save_version,
            schema_version: snap.schema_version,
            rest,
        })
    }
}

fn validate_existing_recorder_metadata(
    rest: &Map<String, Value>,
    binding: &RecorderSessionBinding,
) -> Result<(), SourceError> {
    if let Some(session_value) = rest.get("session") {
        let session = session_value
            .as_object()
            .ok_or_else(|| SourceError::Open("snapshot session is not an object".into()))?;
        check_string_list(
            session.get("active_dlcs"),
            "session.active_dlcs",
            &binding.active_dlcs,
        )?;
        check_string_list(session.get("dlcs"), "session.dlcs", &binding.active_dlcs)?;
        check_string(
            session.get("localisation_language"),
            "session.localisation_language",
            &binding.localisation_language,
        )?;
        check_string(
            session.get("language"),
            "session.language",
            &binding.localisation_language,
        )?;
        check_exact_value(
            session.get("display_variant"),
            "session.display_variant",
            &serde_json::json!(binding.display_variant),
        )?;
    }

    if let Some(world_value) = rest.get("world") {
        let world = world_value
            .as_object()
            .ok_or_else(|| SourceError::Open("snapshot world is not an object".into()))?;
        if let Some(scenario_value) = world.get("scenario") {
            let scenario = scenario_value.as_object().ok_or_else(|| {
                SourceError::Open("snapshot world.scenario is not an object".into())
            })?;
            check_string_list(
                scenario.get("dlcs"),
                "world.scenario.dlcs",
                &binding.active_dlcs,
            )?;
            check_string(
                scenario.get("localisation_language"),
                "world.scenario.localisation_language",
                &binding.localisation_language,
            )?;
            check_string(
                scenario.get("language"),
                "world.scenario.language",
                &binding.localisation_language,
            )?;
        }
    }
    Ok(())
}

fn check_exact_value(
    value: Option<&Value>,
    context: &str,
    expected: &Value,
) -> Result<(), SourceError> {
    if value.is_some_and(|actual| actual != expected) {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict: {context} does not match launch-bound manifest metadata"
        )));
    }
    Ok(())
}

fn check_string_list(
    value: Option<&Value>,
    context: &str,
    expected: &[String],
) -> Result<(), SourceError> {
    let Some(value) = value else {
        return Ok(());
    };
    let array = value.as_array().ok_or_else(|| {
        SourceError::Open(format!(
            "recorder metadata conflict: {context} is not a string array"
        ))
    })?;
    let actual: Vec<String> = array
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                SourceError::Open(format!(
                    "recorder metadata conflict: {context}[{index}] is not a string"
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    if actual != expected {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict at {context}: snapshot {actual:?} != recorder binding {expected:?}"
        )));
    }
    Ok(())
}

fn check_string(value: Option<&Value>, context: &str, expected: &str) -> Result<(), SourceError> {
    let Some(value) = value else {
        return Ok(());
    };
    let actual = value.as_str().ok_or_else(|| {
        SourceError::Open(format!(
            "recorder metadata conflict: {context} is not a string"
        ))
    })?;
    if actual != expected {
        return Err(SourceError::Open(format!(
            "recorder metadata conflict at {context}: snapshot {actual:?} != recorder binding {expected:?}"
        )));
    }
    Ok(())
}

fn bind_or_match(
    object: &mut Map<String, Value>,
    key: &str,
    binding: Value,
) -> Result<(), SourceError> {
    match object.get(key) {
        None => {
            object.insert(key.to_string(), binding);
            Ok(())
        }
        Some(existing) if *existing == binding => Ok(()),
        Some(existing) => Err(SourceError::Open(format!(
            "recorder metadata bind_or_match conflict at session.{key}: snapshot {existing} != recorder binding {binding}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "replay_source_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// R5-H3: the integrity class must travel typed. Its Display is byte-for-
    /// byte the untyped `Open` rendering, so every existing message contract
    /// holds while consumers gain a class to match on.
    #[test]
    fn integrity_is_a_typed_sibling_of_open_with_an_identical_rendering() {
        let message = "entry payload/g/h.json: sha256 aa != manifest bb";
        assert_eq!(
            SourceError::Integrity(message.into()).to_string(),
            SourceError::Open(message.into()).to_string()
        );
        assert!(matches!(
            SourceError::Integrity(message.into()),
            SourceError::Integrity(_)
        ));
    }

    /// A failed digest, a failed CRC, a short payload and a decompression
    /// failure are the integrity class; a malformed/semantic refusal is not.
    /// Classification is a total match on the typed snapshot errors — never a
    /// substring of the message.
    #[test]
    fn load_failures_are_classified_by_type_not_by_message_text() {
        let integrity: Vec<snapshot::LoadError> = vec![
            snapshot::LoadError::Container(snapshot::ContainerError::PayloadIntegrity(
                "entry x: sha256 aa != manifest bb".into(),
            )),
            snapshot::LoadError::Container(snapshot::ContainerError::Zip(
                snapshot::zip::ZipError::PayloadIntegrity("entry x: payload crc32 mismatch".into()),
            )),
            snapshot::LoadError::Container(snapshot::ContainerError::Zip(
                snapshot::zip::ZipError::SizeMismatch {
                    name: "x".into(),
                    declared: 10,
                    got: 9,
                },
            )),
            snapshot::LoadError::Container(snapshot::ContainerError::Zip(
                snapshot::zip::ZipError::Decompress("inflate failed".into()),
            )),
        ];
        for error in &integrity {
            assert!(
                matches!(
                    source_error_from_load(error, "m".into()),
                    SourceError::Integrity(_)
                ),
                "{error} must be the integrity class"
            );
        }

        let not_integrity: Vec<snapshot::LoadError> = vec![
            // A semantic/identity refusal: nothing was authenticated and lost.
            snapshot::LoadError::Container(snapshot::ContainerError::Reject(
                "manifest missing required_features".into(),
            )),
            snapshot::LoadError::Container(snapshot::ContainerError::NoManifest),
            // Structural shortfalls of the ZIP scaffolding: a malformed
            // archive, not payload bytes that failed their digest.
            snapshot::LoadError::Container(snapshot::ContainerError::Zip(
                snapshot::zip::ZipError::Truncated("central directory"),
            )),
            snapshot::LoadError::Directory("anchor escaped its root".into()),
            // A message that CONTAINS integrity words but is not the class.
            snapshot::LoadError::Container(snapshot::ContainerError::Reject(
                "entry x: sha256 field is not lowercase hex".into(),
            )),
        ];
        for error in &not_integrity {
            assert!(
                matches!(
                    source_error_from_load(error, "m".into()),
                    SourceError::Open(_)
                ),
                "{error} must stay the ordinary open class"
            );
        }

        // The pre-existing typed modset signal is unchanged.
        assert!(matches!(
            source_error_from_load(
                &snapshot::LoadError::Container(snapshot::ContainerError::ModsetMismatch {
                    manifest: "a".into(),
                    snapshot: "b".into(),
                }),
                "m".into()
            ),
            SourceError::ModsetMismatch(_)
        ));
    }

    /// Return the body of one top-level `fn <name>(` in this module's source.
    fn function_body(name: &str) -> String {
        // Normalize line endings: include_str! sees the checkout's bytes, and
        // a CRLF checkout would never match the " {\n" body-open pattern.
        let source = include_str!("source.rs").replace("\r\n", "\n");
        let source = source.as_str();
        let start = source
            .find(&format!("\nfn {name}("))
            .unwrap_or_else(|| panic!("{name} must exist in this module"));
        let open = source[start..]
            .find(" {\n")
            .unwrap_or_else(|| panic!("{name} must have a body"))
            + start;
        let bytes = source.as_bytes();
        let mut depth = 0usize;
        for (offset, byte) in bytes[open..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return source[open..open + offset + 1].to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("{name} body is unbalanced");
    }

    /// R5-L4 / R5-L5. Two properties whose violation is invisible to any
    /// fixture this build can construct — the compact ceiling needs a >2 GiB
    /// declared entry, and the per-hour identity recheck on the LAZY spooled
    /// route only fires for an hour a real spool serves. Both are single call
    /// sites in this module, so the call site itself is what the gate pins:
    /// deleting either one fails this test.
    #[test]
    fn both_compact_routes_admit_the_product_ceiling_and_every_prefold_route_proves_identity() {
        for route in [
            "open_compact_stream_with_decoder",
            "open_compact_stream_with_spooled_decoder",
        ] {
            assert!(
                function_body(route).contains("admit_product_compact_stream_bytes("),
                "{route} must apply the shipped compact-playback ceiling before materializing \
                 or spooling the stream"
            );
        }
        // The eager route proves identity for every entry (covered end-to-end
        // by tests/r5_ingest_properties.rs); the lazy route cannot, so it
        // re-proves identity for each hour it actually serves.
        let eager = function_body("validate_prefolded_container_identity");
        assert!(
            eager.contains("for entry in replay.entries()"),
            "the eager prefold route must prove identity for EVERY hour, not just the first"
        );
        let lazy = include_str!("source.rs");
        let lazy_frame = lazy
            .find("impl ReplaySource for SpooledPrefoldedSource")
            .expect("the lazy spooled source must exist");
        let lazy_body = &lazy[lazy_frame..];
        let lazy_end = lazy_body
            .find("\nimpl ")
            .map(|offset| offset + 1)
            .unwrap_or(lazy_body.len());
        assert!(
            lazy_body[..lazy_end].contains("validate_snapshot_container_identity(container,"),
            "the lazy spooled source must re-prove container identity for each hour it serves"
        );
    }

    #[test]
    fn shipped_compact_stream_budget_is_closed_at_two_gibibytes() {
        assert!(admit_product_compact_stream_bytes(PRODUCT_COMPACT_STREAM_MAX_BYTES).is_ok());
        let error = admit_product_compact_stream_bytes(PRODUCT_COMPACT_STREAM_MAX_BYTES + 1)
            .expect_err("one byte beyond the product budget must fail before spooling");
        assert!(error.to_string().contains("2 GiB"));
        assert!(PRODUCT_COMPACT_STREAM_MAX_BYTES < 64 * 1024 * 1024 * 1024);
    }

    #[test]
    fn recorder_recovery_and_incomplete_production_profiles_are_not_playable() {
        let admitted = json!({
            "scheduled_domains": ["fuel_live"],
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
                "gate_problems": [],
            },
            "production_profile": {
                "enabled": true,
                "profile_domains": ["fuel_live"],
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        let full = validate_recorder_product_admission(admitted.as_object().unwrap()).unwrap();
        assert!(
            full.is_full_production_profile(),
            "an enabled profile with three empty lists holds every guarantee"
        );

        let mut recovery = admitted.clone();
        recovery["replay_artifact"]["recovery"] = json!(true);
        recovery["replay_artifact"]["reason"] = json!("gate_failed");
        recovery["replay_artifact"]["gate_problems"] = json!(["domain_missing"]);
        let error = validate_recorder_product_admission(recovery.as_object().unwrap())
            .expect_err("recovery replay must never enter production playback");
        assert!(error.to_string().contains("recovery-only"));

        let mut incomplete = admitted;
        incomplete["production_profile"]["mandatory_domains_not_production_complete"] =
            json!(["weather_live"]);
        let error = validate_recorder_product_admission(incomplete.as_object().unwrap())
            .expect_err("non-complete mandatory production domain must fail closed");
        assert!(
            error
                .to_string()
                .contains("mandatory_domains_not_production_complete")
        );

        let mut incomplete = json!({
            "scheduled_domains": ["fuel_live"],
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
                "gate_problems": [],
            },
            "production_profile": {
                "enabled": true,
                "profile_domains": ["fuel_live", "technology_mio_live"],
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        incomplete["production_profile"]["profile_domains_not_scheduled"] =
            json!(["technology_mio_live"]);
        let error = validate_recorder_product_admission(incomplete.as_object().unwrap())
            .expect_err("incomplete production profile must fail closed");
        assert!(error.to_string().contains("profile_domains_not_scheduled"));

        let missing_positive_authority = json!({
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
            },
            "production_profile": {
                "mandatory_domains_not_production_complete": [],
                "profile_domains_not_scheduled": [],
                "missing_required_static_artifacts": [],
            },
        });
        assert!(
            validate_recorder_product_admission(
                missing_positive_authority.as_object().unwrap()
            )
            .is_err(),
            "present admission authorities must be complete and positive"
        );
    }

    /// kills: dropping the `enabled == false` carve-out (case 1 stops
    /// admitting), carving out `mandatory_domains_not_production_complete`
    /// (case 4 stops refusing), letting the carve-out leak into an enabled
    /// profile (case 5 stops refusing), losing the named shortfall on the way
    /// into provenance (cases 1-3 stop naming it), or accepting a
    /// non-boolean/absent `enabled` or a non-string list element as a false
    /// value (cases 6-7 stop refusing).
    ///
    /// The production change each case is aimed at is the SHORTEST fix someone
    /// under pressure would reach for: "just accept anything when the profile
    /// is off". That is exactly what cases 4 and 5 forbid, and they are the
    /// reason this is a carve-out and not a loosening.
    #[test]
    fn late_attach_captures_are_admitted_with_their_shortfall_named() {
        // The two rosters the declared difference is a difference OF are built
        // consistently: the profile is a fixed pair plus whatever this case
        // says is unscheduled, and the schedule is the fixed pair.
        let late_attach = |mandatory: Value, domains: Value, artifacts: Value, enabled: bool| {
            let scheduled = ["fuel_live", "production_live"];
            let mut profile_domains = scheduled.iter().map(|d| json!(d)).collect::<Vec<_>>();
            profile_domains.extend(domains.as_array().expect("domain list").iter().cloned());
            json!({
                "scheduled_domains": scheduled,
                "replay_artifact": {
                    "recovery": false,
                    "reason": "gate_passed",
                    "gate_problems": [],
                },
                "production_profile": {
                    "enabled": enabled,
                    "profile_domains": profile_domains,
                    "mandatory_domains_not_production_complete": mandatory,
                    "profile_domains_not_scheduled": domains,
                    "missing_required_static_artifacts": artifacts,
                },
            })
        };

        // 1. The shipped attach-and-play product: profile off, full runtime
        // roster scheduled, one required static artifact not carried. Admitted,
        // with the profile bit named as not held.
        //
        // `RequiredStaticArtifacts` is deliberately absent from what THIS
        // function returns: it is decided against the package at the roster
        // join, not from the declaration, so that one guarantee and the list
        // naming it cannot disagree. The declaration read here is what the join
        // reconciles the package against, and the guarantee itself is covered
        // by `tests/world_timeline.rs` end to end.
        let product = late_attach(json!([]), json!([]), json!(["focus_defs_static"]), false);
        let admission =
            validate_recorder_product_admission(product.as_object().unwrap()).expect(
                "a late-attach capture must open; --profile production is early-launch-only",
            );
        assert!(!admission.is_full_production_profile());
        assert_eq!(
            admission.guarantees_not_held,
            vec![ProductionProfileGuarantee::ProductionProfileEnabled]
        );
        assert_eq!(
            admission.missing_required_static_artifacts,
            vec!["focus_defs_static".to_string()],
            "the declaration is read verbatim here; the roster join replaces it with the \
             package's own answer after proving the two agree"
        );
        assert!(admission.profile_domains_not_scheduled.is_empty());

        // 2. A narrower probe additionally leaves profile domains unscheduled.
        // Still an accounted absence, still admitted, now naming all three.
        let probe = late_attach(
            json!([]),
            json!(["decision_panel_live"]),
            json!(["focus_defs_static"]),
            false,
        );
        let admission =
            validate_recorder_product_admission(probe.as_object().unwrap()).unwrap();
        assert_eq!(
            admission.guarantees_not_held,
            vec![
                ProductionProfileGuarantee::ProductionProfileEnabled,
                ProductionProfileGuarantee::FullProfileDomainSchedule,
            ]
        );
        assert_eq!(
            admission.profile_domains_not_scheduled,
            vec!["decision_panel_live".to_string()]
        );

        // 3. A capture that scheduled and carried everything but never claimed
        // the profile loses exactly one guarantee, and names nothing missing.
        let admission =
            validate_recorder_product_admission(
                late_attach(json!([]), json!([]), json!([]), false)
                    .as_object()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            admission.guarantees_not_held,
            vec![ProductionProfileGuarantee::ProductionProfileEnabled]
        );

        // 4. THE LINE. A domain whose producer is not production-complete is a
        // statement about the recorder BUILD, not about how this capture
        // attached, so turning the profile off does not carve it out.
        let error = validate_recorder_product_admission(
            late_attach(json!(["weather_live"]), json!([]), json!([]), false)
                .as_object()
                .unwrap(),
        )
        .expect_err("a not-production-complete producer refuses whether or not the profile ran");
        assert!(error
            .to_string()
            .contains("mandatory_domains_not_production_complete"));

        // 5. The carve-out never reaches a capture that CLAIMS the profile.
        let error = validate_recorder_product_admission(
            late_attach(json!([]), json!([]), json!(["focus_defs_static"]), true)
                .as_object()
                .unwrap(),
        )
        .expect_err("a capture claiming the production profile must satisfy all of it");
        assert!(error
            .to_string()
            .contains("missing_required_static_artifacts"));

        // 6. A malformed authority is malformed, not permissive: an absent or
        // non-boolean `enabled` is refused rather than read as `false`.
        //
        // The ABSENT case is separate from the non-boolean ones and is checked
        // on an otherwise fully valid manifest. Folding it into a fixture that
        // is also missing `replay_artifact.gate_problems` would let a change
        // that defaults only an absent `enabled` to `false` pass, because that
        // fixture fails earlier for an unrelated reason (Codex round 1, LOW).
        let mut absent = late_attach(json!([]), json!([]), json!([]), false);
        absent["production_profile"]
            .as_object_mut()
            .unwrap()
            .remove("enabled");
        let error = validate_recorder_product_admission(absent.as_object().unwrap())
            .expect_err("an absent enabled is a malformed authority, not a false one");
        assert!(error
            .to_string()
            .contains("enabled is absent or is not a boolean"));

        for broken in [json!(null), json!("false"), json!(0)] {
            let mut manifest = late_attach(json!([]), json!([]), json!([]), false);
            manifest["production_profile"]["enabled"] = broken.clone();
            let error = validate_recorder_product_admission(manifest.as_object().unwrap())
                .expect_err("a non-boolean enabled is a malformed authority");
            assert!(
                error.to_string().contains("enabled is absent or is not a boolean"),
                "{broken} was not refused as a malformed authority: {error}"
            );
        }

        // 7. The lists now reach provenance, so their elements are typed.
        let mut manifest = late_attach(json!([]), json!([]), json!([]), false);
        manifest["production_profile"]["profile_domains_not_scheduled"] = json!([7]);
        let error = validate_recorder_product_admission(manifest.as_object().unwrap())
            .expect_err("a non-string list element is a malformed authority");
        assert!(error
            .to_string()
            .contains("profile_domains_not_scheduled[0] is not a string"));

        // 7b. THE DECLARED DIFFERENCE IS CHECKED AGAINST THE ROSTERS IT IS A
        // DIFFERENCE OF, in the one direction replay can compute. Declaring a
        // domain unscheduled that the manifest's own schedule names is a
        // contradiction inside one document, and no alias can produce it: both
        // the declaration and `profile_domains` are canonical wire names, so a
        // hit here means the schedule spelled it canonically too.
        let mut overstated = late_attach(json!([]), json!(["fuel_live"]), json!([]), false);
        overstated["production_profile"]["profile_domains"] =
            json!(["fuel_live", "production_live"]);
        let error = validate_recorder_product_admission(overstated.as_object().unwrap())
            .expect_err("a scheduled domain may not be declared unscheduled");
        assert!(error
            .to_string()
            .contains("which the manifest's own scheduled_domains carries"));

        // THE OTHER DIRECTION IS DELIBERATELY NOT CHECKED, and this case is
        // here to say so rather than to leave the gap unnamed. A manifest that
        // lists a profile domain, omits it from `scheduled_domains` and still
        // declares nothing unscheduled is ADMITTED: `scheduled_domains` is the
        // operator's vocabulary and admits legacy aliases the extractor's
        // registry resolves (`state_ownership` is the same domain as the
        // profile's `map_control`), and the replay workspace may not depend on
        // that registry. Computing the difference here would refuse a
        // legitimate aliased full-profile recording — a false refusal, which is
        // worse. The closing step is producer-side and is named in the function
        // docs (Codex round 5, HIGH).
        let mut understated = late_attach(json!([]), json!([]), json!([]), false);
        understated["production_profile"]["profile_domains"] =
            json!(["fuel_live", "production_live", "decision_panel_live"]);
        let admission = validate_recorder_product_admission(understated.as_object().unwrap())
            .expect("under-declaration is not computable here; see the function docs");
        assert_eq!(
            admission.guarantees_not_held,
            vec![ProductionProfileGuarantee::ProductionProfileEnabled],
            "if this ever starts naming FullProfileDomainSchedule, the difference became \
             computable and the comment above is stale"
        );

        // An aliased schedule must NOT be refused. This is the false refusal
        // the check above was rewritten to avoid: `map_control` is a profile
        // domain, the schedule carries its legacy alias, and the difference is
        // correctly declared empty.
        let mut aliased = late_attach(json!([]), json!([]), json!([]), true);
        aliased["production_profile"]["profile_domains"] = json!(["map_control"]);
        aliased["scheduled_domains"] = json!(["state_ownership"]);
        validate_recorder_product_admission(aliased.as_object().unwrap())
            .expect("a legitimate full-profile capture scheduling a legacy alias must open");

        // A declared name outside the profile roster is named as such.
        let mut foreign = late_attach(json!([]), json!([]), json!([]), false);
        foreign["production_profile"]["profile_domains_not_scheduled"] =
            json!(["not_a_profile_domain"]);
        let error = validate_recorder_product_admission(foreign.as_object().unwrap())
            .expect_err("a name outside the profile roster must refuse");
        assert!(error
            .to_string()
            .contains("is not in production_profile.profile_domains"));

        // Repeats in either roster are malformed, not tolerated by set logic.
        let mut repeated = late_attach(json!([]), json!([]), json!([]), false);
        repeated["scheduled_domains"] = json!(["fuel_live", "fuel_live", "production_live"]);
        let error = validate_recorder_product_admission(repeated.as_object().unwrap())
            .expect_err("a repeated scheduled domain is malformed");
        assert!(error.to_string().contains("repeats a domain name"));

        // 8. No `production_profile` authority at all: nothing is claimed, so
        // the profile bit is not held and nothing is named missing.
        let unstated = json!({
            "replay_artifact": {
                "recovery": false,
                "reason": "gate_passed",
                "gate_problems": [],
            },
        });
        let admission =
            validate_recorder_product_admission(unstated.as_object().unwrap()).unwrap();
        assert_eq!(admission, RecorderProductAdmission::unstated());
        assert_eq!(admission.authority, ProductionProfileAuthority::Unstated);
        // Both MANIFEST-ONLY guarantees, not just the profile bit: an absent
        // authority evidences the schedule exactly as little as it evidences
        // the profile. `RequiredStaticArtifacts` is not here because it is
        // verifiable against the package and is decided there.
        assert_eq!(
            admission.guarantees_not_held,
            vec![
                ProductionProfileGuarantee::ProductionProfileEnabled,
                ProductionProfileGuarantee::FullProfileDomainSchedule,
            ]
        );

        // 9. The three authority states are distinguishable, so "said nothing"
        // is never read as "said it fell short" or the reverse.
        assert_eq!(
            validate_recorder_product_admission(
                late_attach(json!([]), json!([]), json!([]), false)
                    .as_object()
                    .unwrap(),
            )
            .unwrap()
            .authority,
            ProductionProfileAuthority::Disabled
        );
        assert_eq!(
            validate_recorder_product_admission(
                late_attach(json!([]), json!([]), json!([]), true)
                    .as_object()
                    .unwrap(),
            )
            .unwrap()
            .authority,
            ProductionProfileAuthority::Enabled
        );
    }

    #[test]
    fn recorder_compact_stream_identity_must_match_selected_role() {
        let manifest = json!({
            "stream_storage": {
                "decoded_bytes": 4096,
                "decoded_sha256": "a".repeat(64),
            },
            "replay_entries": {
                "stream": "collector_raw/collector_stream.bin",
                "stream_role": "collector_compact_stream",
            },
        });
        let manifest = manifest.as_object().unwrap();
        validate_recorder_compact_stream_join(
            "payload/game/collector_raw/collector_stream.bin",
            4096,
            &"a".repeat(64),
            "game",
            Some(manifest),
        )
        .unwrap();

        let error = validate_recorder_compact_stream_join(
            "payload/game/collector_raw/collector_stream.bin",
            4097,
            &"a".repeat(64),
            "game",
            Some(manifest),
        )
        .expect_err("different role bytes must not enter prefolding");
        assert!(error.to_string().contains("decoded identity"));
    }

    fn portable_content_manifest_fixture() -> portable_content_manifest::ContentManifest {
        let mut manifest = portable_content_manifest::ContentManifest {
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: "a".repeat(64),
            source_game: portable_content_manifest::SourceGameIdentity {
                version: "1.0".into(),
                build: "fixture".into(),
                executable_sha256: "b".repeat(64),
                build_profile: "fixture-profile".into(),
            },
            source_profile: portable_content_manifest::SourceProfileIdentity {
                profile_id: "fixture-profile".into(),
                playset_id: "fixture-playset".into(),
                selection_sha256: "0".repeat(64),
            },
            effective_vfs_manifest_sha256: "c".repeat(64),
            ordered_layers: Vec::new(),
            dlc_selection: Vec::new(),
            localisation_language: "english".into(),
            required_settings: BTreeMap::from([
                ("physical_width".into(), "1920".into()),
                ("physical_height".into(), "1080".into()),
                ("min_gui_width".into(), "1920".into()),
                ("min_gui_height".into(), "1080".into()),
                ("gui_scale_f32_bits".into(), 1.0f32.to_bits().to_string()),
                ("refresh_rate_hz".into(), "60".into()),
                ("max_refresh_rate_hz".into(), "unavailable".into()),
                ("fullscreen".into(), "false".into()),
                ("borderless".into(), "true".into()),
                ("display_mode".into(), "borderless_fullscreen".into()),
                ("settings_file_sha256".into(), "e".repeat(64)),
                // How well the settings are proven travels in the same map
                // that carries them, so a replay reproducing them cannot miss
                // that a recording only OBSERVED the settings on disk rather
                // than proving the game read them.
                ("settings_authority".into(), "pre_launch_bracketed".into()),
            ]),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 =
            manifest.computed_selection_sha256().unwrap();
        manifest.with_computed_digest().unwrap()
    }

    #[test]
    fn recorder_and_container_content_authorities_must_join_exactly() {
        let container = portable_content_manifest_fixture();
        let recorder = json!({
            "portable_content_manifest": serde_json::to_value(&container).unwrap(),
            "modset": {
                "game": {"exe_sha256": container.source_game.executable_sha256.clone()},
                "settings_binding": {
                    "authority": "pre_launch_bracketed",
                    "launch_proven": true,
                    "errors": [],
                    "language": container.localisation_language.clone(),
                    "display_variant": {
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "min_gui_width": 1920,
                        "min_gui_height": 1080,
                        "gui_scale_f32_bits": 1.0f32.to_bits(),
                        "refresh_rate_hz": 60,
                        "max_refresh_rate_hz": null,
                        "fullscreen": false,
                        "borderless": true,
                        "display_mode": "borderless_fullscreen",
                    },
                    "attach_settings_sha256": "e".repeat(64),
                    "seal_settings_sha256": null,
                    "endpoint_semantics_equal": null
                },
            }
        })
        .as_object()
        .unwrap()
        .clone();
        validate_recorder_content_manifest_join(&recorder, Some(&container)).unwrap();

        let mut wrong_launch = recorder.clone();
        wrong_launch
            .get_mut("modset")
            .unwrap()
            .get_mut("game")
            .unwrap()["exe_sha256"] = json!("d".repeat(64));
        let error = validate_recorder_content_manifest_join(&wrong_launch, Some(&container))
            .expect_err("content authority must join the recorded launch executable");
        assert!(error.to_string().contains("executable disagrees"));

        let mut wrong_display = recorder.clone();
        wrong_display
            .get_mut("modset")
            .unwrap()
            .get_mut("settings_binding")
            .unwrap()
            .get_mut("display_variant")
            .unwrap()["physical_width"] = json!(2560);
        let error = validate_recorder_content_manifest_join(&wrong_display, Some(&container))
            .expect_err("content settings must join the recorded display variant");
        assert!(error.to_string().contains("display variant disagrees"));

        let missing = Map::new();
        let error = validate_recorder_content_manifest_join(&missing, Some(&container))
            .expect_err("container-only authority must fail closed");
        assert!(error.to_string().contains("recorder manifest omits"));

        let mut different = container.clone();
        different.product_modset_sha256 = "d".repeat(64);
        different.manifest_digest_sha256 = None;
        different = different.with_computed_digest().unwrap();
        let error = validate_recorder_content_manifest_join(&recorder, Some(&different))
            .expect_err("different valid authorities must fail closed");
        assert!(error.to_string().contains("disagrees"));

        let error = validate_recorder_content_manifest_join(&recorder, None)
            .expect_err("recorder-only authority must fail closed");
        assert!(error.to_string().contains("container manifest omits"));
    }

    #[test]
    fn private_prefold_spool_cleanup_is_identity_gated() {
        let base = temp_root("prefold_spool_owner");
        let owner = PrefoldSpoolOwner::create(&base).unwrap();
        let owned_root = owner.root().to_path_buf();
        drop(owner);
        assert!(!owned_root.exists(), "intact private spool must be cleaned");

        let owner = PrefoldSpoolOwner::create(&base).unwrap();
        let preserved_root = owner.root().to_path_buf();
        fs::write(preserved_root.join(PREFOLD_SPOOL_MARKER), "different-owner").unwrap();
        drop(owner);
        assert!(
            preserved_root.exists(),
            "identity mismatch must preserve evidence instead of recursive deletion"
        );
        portable_test_fs_retry::remove_dir_all(&preserved_root).unwrap();
        portable_test_fs_retry::remove_dir_all(&base).unwrap();
    }

    fn write_playable_store(root: &Path, game: &str, recorder: bool, label: &str) {
        let game_dir = root.join(game);
        fs::create_dir_all(game_dir.join("recorder")).unwrap();
        fs::write(
            game_dir.join("index.json"),
            serde_json::to_vec(&json!({
                "game_id": game,
                "snapshots": [{"hour_index": 1, "file": "000000000001.snapshot.json"}]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            game_dir.join("000000000001.snapshot.json"),
            serde_json::to_vec(&json!({
                "game_id": game,
                "hour_index": 1,
                "marker": label
            }))
            .unwrap(),
        )
        .unwrap();
        if recorder {
            fs::write(
                game_dir.join("timeline.json"),
                br#"{"session_manifest":"recorder/native_session_manifest.json"}"#,
            )
            .unwrap();
            fs::write(
                game_dir
                    .join("recorder")
                    .join("native_session_manifest.json"),
                serde_json::to_vec(&json!({
                    "modset": {
                        "resolved": true,
                        "errors": [],
                        "dlc": {"enabled": ["dlc/a.dlc"]},
                        "dlc_display_names": {"dlc/a.dlc": "DLC A"},
                        "settings_binding": {
                            "authority": "pre_launch_bracketed",
                            "launch_proven": true,
                            "errors": [],
                            "language": "l_english",
                            "display_variant": {
                                "physical_width": 2752, "physical_height": 1152,
                                "min_gui_width": 1280, "min_gui_height": 720,
                                "gui_scale_f32_bits": 1.0f32.to_bits(),
                                "refresh_rate_hz": 60, "max_refresh_rate_hz": 75,
                                "fullscreen": false, "borderless": true,
                                "display_mode": "borderless_fullscreen",
                            },
                            "attach_settings_sha256": "aa".repeat(32),
                            "seal_settings_sha256": null,
                            "endpoint_semantics_equal": null
                        },
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        } else {
            fs::write(game_dir.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
        }
    }

    fn write_portable_recorder_fixture(
        path: &Path,
        game: &str,
        label: &str,
        campaign_identity: Option<Value>,
    ) {
        let content = portable_content_manifest_fixture();
        let absent_static_artifacts = REQUIRED_PACKAGED_STATIC_ARTIFACTS
            .iter()
            .map(|(name, path)| {
                json!({
                    "name": name,
                    "path": path,
                    "present": false,
                })
            })
            .collect::<Vec<_>>();
        let mut recorder_manifest = json!({
            "replay_game_id": game,
            "replay_packaging": {
                "static_artifacts": [],
            },
            snapshot::RAW_STREAM_PACKAGING_FIELD:
                snapshot::RawStreamPackaging::Omitted.as_str(),
            "portable_content_manifest": serde_json::to_value(&content).unwrap(),
            "modset": {
                "resolved": true,
                "errors": [],
                "game": {"exe_sha256": content.source_game.executable_sha256.clone()},
                "dlc": {"enabled": []},
                "dlc_display_names": {},
                "settings_binding": {
                    "authority": "pre_launch_bracketed",
                    "launch_proven": true,
                    "errors": [],
                    "language": content.localisation_language.clone(),
                    "display_variant": {
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "min_gui_width": 1920,
                        "min_gui_height": 1080,
                        "gui_scale_f32_bits": 1.0f32.to_bits(),
                        "refresh_rate_hz": 60,
                        "max_refresh_rate_hz": null,
                        "fullscreen": false,
                        "borderless": true,
                        "display_mode": "borderless_fullscreen",
                    },
                    "attach_settings_sha256": "e".repeat(64),
                    "seal_settings_sha256": null,
                    "endpoint_semantics_equal": null
                },
            },
        });
        if let Some(campaign_identity) = campaign_identity {
            recorder_manifest["campaign_identity"] = campaign_identity;
        }
        let entries = vec![
            snapshot::PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game,
                    "schema_version": "1",
                    "host_fold_skipped": false,
                    "static_artifacts": absent_static_artifacts,
                    // This fixture is a per-hour-snapshot recording carrying no
                    // raw stream entry, so `omitted` is its truthful
                    // composition; all three documents must say the same.
                    snapshot::RAW_STREAM_PACKAGING_FIELD:
                        snapshot::RawStreamPackaging::Omitted.as_str(),
                    "snapshots": [{
                        "hour_index": 1,
                        "file": "000000000001.snapshot.json",
                    }],
                }))
                .unwrap(),
                role: "index",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/timeline.json"),
                bytes: serde_json::to_vec(&json!({
                    "format": COMPACT_STREAM_TIMELINE_FORMAT,
                    "stream_format": "compact",
                    "raw_stream_role": COMPACT_STREAM_ROLE,
                    "session_manifest": "recorder/native_session_manifest.json",
                    "session_manifest_role": "recorder_manifest",
                    snapshot::RAW_STREAM_PACKAGING_FIELD:
                        snapshot::RawStreamPackaging::Omitted.as_str(),
                }))
                .unwrap(),
                role: "timeline",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/recorder/native_session_manifest.json"),
                bytes: serde_json::to_vec(&recorder_manifest).unwrap(),
                role: "recorder_manifest",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/000000000001.snapshot.json"),
                bytes: serde_json::to_vec(&json!({
                    "game_id": game,
                    "schema_version": "1",
                    "hour_index": 1,
                    "marker": label,
                }))
                .unwrap(),
                role: "snapshot",
                hour_index: Some(1),
            },
        ];
        let packed = snapshot::pack(
            game,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        fs::write(path, packed).unwrap();
    }

    /// Returns a retained read handle on the container it just wrote.
    ///
    /// Every caller opens it on the next statement, and on Windows that is
    /// exactly when a real-time scanner still holds it. Bind the result
    /// (`let _guard = ...`) so it lives for the length of the test.
    #[must_use = "the scanner guard must be held until the recording is opened"]
    fn write_portable_binary_replay_fixture(
        path: &Path,
        valid_checkpoint_role: bool,
    ) -> std::fs::File {
        write_portable_binary_replay_fixture_with_composition(
            path,
            valid_checkpoint_role,
            snapshot::RawStreamPackaging::Packaged,
        )
    }

    /// Build the same certified two-hour packaged-cache recording under either
    /// composition. The cache, the index descriptor and the recorder identity
    /// are byte-for-byte the same in both; only whether the raw H4CR entry is
    /// packaged (and what the documents declare) changes.
    #[must_use = "the scanner guard must be held until the recording is opened"]
    fn write_portable_binary_replay_fixture_with_composition(
        path: &Path,
        valid_checkpoint_role: bool,
        packaging: snapshot::RawStreamPackaging,
    ) -> std::fs::File {
        let game = "binary-game";
        let source_bytes = b"H4CRcanonical-world-source";
        let source_sha256 = snapshot::sha256::Sha256::hex(source_bytes);
        let worlds = [
            serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "hour_index": 10,
                "date": "1936.1.1.10",
                "countries": {"FRA": {"stability": 0.50}},
                "states": {},
            }))
            .unwrap(),
            serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "hour_index": 11,
                "date": "1936.1.1.11",
                "countries": {"FRA": {"stability": 0.55}},
                "states": {},
            }))
            .unwrap(),
        ];
        let first_end = 8;
        let mut encoder =
            snapshot::ReplayCacheEncoder::new(Vec::new(), &source_sha256, 2).unwrap();
        encoder
            .push(snapshot::CacheBuildHour {
                hour_index: 10,
                date: "1936.1.1.10".into(),
                h4cr_start_offset: 0,
                h4cr_end_offset: first_end,
                world_json: worlds[0].clone(),
            })
            .unwrap();
        encoder
            .push(snapshot::CacheBuildHour {
                hour_index: 11,
                date: "1936.1.1.11".into(),
                h4cr_start_offset: first_end,
                h4cr_end_offset: source_bytes.len() as u64,
                world_json: worlds[1].clone(),
            })
            .unwrap();
        let (checkpoint_bytes, replay_index) = encoder
            .finish(
                game,
                "1",
                snapshot::CacheIdentityMode::Legacy,
                source_bytes.len() as u64,
            )
            .unwrap();
        let hour_index_bytes = replay_index.to_bytes().unwrap();
        let hour_index_sha256 = snapshot::sha256::Sha256::hex(&hour_index_bytes);
        let checkpoint_sha256 = snapshot::sha256::Sha256::hex(&checkpoint_bytes);
        let absent_static_artifacts = REQUIRED_PACKAGED_STATIC_ARTIFACTS
            .iter()
            .map(|(name, artifact_path)| {
                json!({
                    "name": name,
                    "path": artifact_path,
                    "present": false,
                })
            })
            .collect::<Vec<_>>();
        let index_bytes = serde_json::to_vec(&json!({
            "game_id": game,
            "schema_version": "1",
            "host_fold_skipped": false,
            "production_json_snapshots_omitted": true,
            "canonical_world_source": snapshot::COMPACT_STREAM_ROLE,
            snapshot::RAW_STREAM_PACKAGING_FIELD: packaging.as_str(),
            "snapshots": [],
            "static_artifacts": absent_static_artifacts,
            "replay_decoder": {
                "schema": "hoi4.replay_decoder.binary_seek.v1",
                "hour_index": {
                    "role": snapshot::HOUR_INDEX_ROLE,
                    "path": snapshot::HOUR_INDEX_FILE,
                    "bytes": hour_index_bytes.len(),
                    "sha256": hour_index_sha256,
                    "hours": 2,
                    "source_bytes": source_bytes.len(),
                    "source_sha256": source_sha256,
                },
                "checkpoints": {
                    "role": snapshot::CHECKPOINT_CACHE_ROLE,
                    "path": snapshot::CHECKPOINT_CACHE_FILE,
                    "bytes": checkpoint_bytes.len(),
                    "sha256": checkpoint_sha256,
                    "checkpoint_count": 1,
                    "checkpoint_interval_hours": 2,
                    "max_seek_deltas": 1,
                },
                "legacy_json_bytes_avoided": worlds.iter().map(Vec::len).sum::<usize>(),
                "exact_reconstruction_parity": true,
                "bounded_seek_certified": true,
            },
        }))
        .unwrap();
        let content = portable_content_manifest_fixture();
        let mut replay_entries = json!({
            "stream_role": snapshot::COMPACT_STREAM_ROLE,
        });
        if packaging.requires_stream_entry() {
            replay_entries.as_object_mut().unwrap().insert(
                "stream".to_string(),
                json!("collector_raw/collector_stream.bin"),
            );
        }
        let recorder_manifest = serde_json::to_vec(&json!({
            "replay_game_id": game,
            "replay_packaging": {"static_artifacts": []},
            snapshot::RAW_STREAM_PACKAGING_FIELD: packaging.as_str(),
            "replay_entries": replay_entries,
            "stream_storage": {
                "decoded_bytes": source_bytes.len(),
                "decoded_sha256": source_sha256,
            },
            "portable_content_manifest": serde_json::to_value(&content).unwrap(),
            "modset": {
                "resolved": true,
                "errors": [],
                "game": {"exe_sha256": content.source_game.executable_sha256.clone()},
                "dlc": {"enabled": []},
                "dlc_display_names": {},
                "settings_binding": {
                    "authority": "pre_launch_bracketed",
                    "launch_proven": true,
                    "errors": [],
                    "language": content.localisation_language.clone(),
                    "display_variant": {
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "min_gui_width": 1920,
                        "min_gui_height": 1080,
                        "gui_scale_f32_bits": 1.0f32.to_bits(),
                        "refresh_rate_hz": 60,
                        "max_refresh_rate_hz": null,
                        "fullscreen": false,
                        "borderless": true,
                        "display_mode": "borderless_fullscreen",
                    },
                    "attach_settings_sha256": "e".repeat(64),
                    "seal_settings_sha256": null,
                    "endpoint_semantics_equal": null
                },
            },
        }))
        .unwrap();
        let mut entries = vec![
            snapshot::PackEntry {
                path: format!("payload/{game}/index.json"),
                bytes: index_bytes,
                role: "index",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/timeline.json"),
                bytes: {
                    let mut timeline_value = json!({
                        "format": COMPACT_STREAM_TIMELINE_FORMAT,
                        "stream_format": "compact",
                        "raw_stream_role": snapshot::COMPACT_STREAM_ROLE,
                        "session_manifest": "recorder/native_session_manifest.json",
                        "session_manifest_role": "recorder_manifest",
                        snapshot::RAW_STREAM_PACKAGING_FIELD: packaging.as_str(),
                    });
                    if packaging.requires_stream_entry() {
                        timeline_value.as_object_mut().unwrap().insert(
                            "stream".to_string(),
                            json!("collector_raw/collector_stream.bin"),
                        );
                    }
                    serde_json::to_vec(&timeline_value).unwrap()
                },
                role: "timeline",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/recorder/native_session_manifest.json"),
                bytes: recorder_manifest,
                role: "recorder_manifest",
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/{}", snapshot::HOUR_INDEX_FILE),
                bytes: hour_index_bytes,
                role: snapshot::HOUR_INDEX_ROLE,
                hour_index: None,
            },
            snapshot::PackEntry {
                path: format!("payload/{game}/{}", snapshot::CHECKPOINT_CACHE_FILE),
                bytes: checkpoint_bytes,
                role: if valid_checkpoint_role {
                    snapshot::CHECKPOINT_CACHE_ROLE
                } else {
                    "wrong_checkpoint_role"
                },
                hour_index: None,
            },
        ];
        if packaging.requires_stream_entry() {
            entries.push(snapshot::PackEntry {
                path: format!("payload/{game}/collector_raw/collector_stream.bin"),
                bytes: source_bytes.to_vec(),
                role: snapshot::COMPACT_STREAM_ROLE,
                hour_index: None,
            });
        }
        let packed = snapshot::pack(
            game,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        fs::write(path, packed).unwrap();
        // The caller opens this on its next statement; the guard it holds is
        // what keeps a scanner from getting there first.
        portable_test_fs_retry::open(path).expect("hold the fixture open across the scanner's window")
    }

    #[test]
    fn packaged_binary_replay_reconstructs_exact_hours_without_json_snapshots() {
        let root = temp_root("binary_replay");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture(&recording, true);

        let source = open_recording(&recording).unwrap();
        assert_eq!(source.game_id(), "binary-game");
        assert_eq!(source.hours(), &[10, 11]);
        assert_eq!(
            source
                .prefold_receipt()
                .expect("binary replay must retain its H4CR receipt")
                .source_len(),
            b"H4CRcanonical-world-source".len() as u64
        );
        let frame = source.frame(11).unwrap();
        assert_eq!(frame.hour_index, 11);
        assert_eq!(frame.date.as_deref(), Some("1936.1.1.11"));
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.55));
        assert!(source.frame(12).is_err());
        drop(source);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// A cache-only recording replays. The raw stream is not in the container
    /// at all, and every hour still reconstructs from the delta cache, with the
    /// H4CR source receipt intact — because the cache is bound to its source by
    /// LENGTH and SHA-256, and those come from the authenticated recorder
    /// manifest rather than from the stream's own bytes.
    ///
    /// This is the test that says the default composition is usable: if it
    /// fails, stream-off captures are unreplayable and the flag is not
    /// shippable.
    ///
    /// Production change that would make this fail: restoring the unconditional
    /// `select(COMPACT_STREAM_ROLE)` in `open_packaged_replay_cache`, or
    /// sourcing the cache binding from the stream entry again.
    #[test]
    fn cache_only_recording_replays_every_hour_without_the_raw_stream() {
        let root = temp_root("binary_replay_cache_only");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Omitted,
        );

        // The container really does not carry the stream.
        let container =
            snapshot::Container::open(&recording, &snapshot::ExpectedIdentity::default()).unwrap();
        assert!(container
            .entries_by_role(snapshot::COMPACT_STREAM_ROLE)
            .is_empty());
        drop(container);

        let source = open_recording(&recording).unwrap();
        assert_eq!(source.game_id(), "binary-game");
        assert_eq!(source.hours(), &[10, 11]);
        // The source binding survives the stream's absence, byte-for-byte.
        assert_eq!(
            source
                .prefold_receipt()
                .expect("a cache-only recording still carries its H4CR source receipt")
                .source_len(),
            b"H4CRcanonical-world-source".len() as u64
        );
        // Hours reconstruct identically to the stream-on fixture.
        let frame = source.frame(11).unwrap();
        assert_eq!(frame.hour_index, 11);
        assert_eq!(frame.date.as_deref(), Some("1936.1.1.11"));
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.55));
        assert!(source.frame(12).is_err());
        drop(source);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// A decoder that must never be reached. The packaged-cache route consumes
    /// the caller's spool BASE but not its decoder, and this proves it.
    struct UnusedSpoolDecoder;

    impl snapshot::CertifiedPrefoldSpoolDecoder for UnusedSpoolDecoder {
        fn decoder_id(&self) -> &'static str {
            "test.packaged-cache.never-decodes"
        }

        fn decode_certified_spool(
            &self,
            _request: snapshot::CertifiedPrefoldSpoolRequest,
        ) -> Result<snapshot::PrefoldedSpool, snapshot::PrefoldDecodeError> {
            panic!("a packaged certified cache must never be folded from the raw stream");
        }
    }

    /// Opening a packaged cache through a caller-owned spool base reads it from
    /// disk instead of holding it, gives byte-identical hours either way, and
    /// leaves nothing behind.
    ///
    /// The resident read is what fails on a real recording — capture t32's cache
    /// is 2,209,767,138 bytes — so this route is the one the product takes. The
    /// fixture is small on purpose; what is under test is that the two routes
    /// agree and that the private spool is removed, not that a big file fits.
    ///
    /// Production change that would make this fail: making the spooled route
    /// return anything the resident route does not, dropping the spool owner
    /// before the cache handle (Windows will not delete an open file), or
    /// forgetting to admit the spooled cache name in the owner's identity check
    /// so the directory is disowned and leaks.
    #[test]
    fn a_packaged_cache_opened_through_a_spool_base_matches_the_resident_read() {
        let root = temp_root("binary_replay_cache_spooled");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Omitted,
        );
        let spool_base = root.join("spool");
        fs::create_dir(&spool_base).unwrap();

        let resident = open_recording(&recording).unwrap();
        let spooled =
            open_recording_with_spooled_decoder(&recording, &spool_base, &UnusedSpoolDecoder)
                .unwrap();
        assert_eq!(spooled.game_id(), resident.game_id());
        assert_eq!(spooled.hours(), resident.hours());
        for hour in resident.hours().to_vec() {
            let expected = resident.frame(hour).unwrap();
            let observed = spooled.frame(hour).unwrap();
            assert_eq!(observed.hour_index, expected.hour_index);
            assert_eq!(observed.date, expected.date);
            assert_eq!(observed.rest, expected.rest, "hour {hour} differs by route");
        }

        // While the source is alive the cache really is on disk, in a private
        // directory under the caller's base.
        let spooled_caches: Vec<PathBuf> = fs::read_dir(&spool_base)
            .unwrap()
            .map(|entry| entry.unwrap().path().join("checkpoints.h4cp"))
            .filter(|path| path.is_file())
            .collect();
        assert_eq!(
            spooled_caches.len(),
            1,
            "the spooled route must materialize exactly one private cache"
        );

        drop(spooled);
        assert_eq!(
            fs::read_dir(&spool_base).unwrap().count(),
            0,
            "dropping the source must remove its private spool"
        );
        drop(resident);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// Flip one byte of the STORED H4CP entry inside an already-packed
    /// recording, at `offset` within the cache artifact.
    ///
    /// POST-SEAL ROT: everything the container says about the entry — its
    /// length, its digest, its CRC — still describes what the recorder sealed,
    /// and only the bytes on disk have moved. Damaging the cache before packing
    /// could not model this, because the packer would have derived its own
    /// account from the damaged bytes.
    ///
    /// The entry is located by its own bytes rather than by parsing the ZIP,
    /// and the occurrence is required to be unique so this can never damage
    /// some other entry that happened to contain the same run.
    fn rot_sealed_cache_byte_in_recording(recording: &Path, cache_bytes: &[u8], offset: usize) {
        let mut packed = fs::read(recording).unwrap();
        let positions: Vec<usize> = packed
            .windows(cache_bytes.len())
            .enumerate()
            .filter(|(_, window)| *window == cache_bytes)
            .map(|(position, _)| position)
            .collect();
        assert_eq!(
            positions.len(),
            1,
            "the sealed cache must appear exactly once in the packed container"
        );
        packed[positions[0] + offset] ^= 0xff;
        fs::write(recording, &packed).unwrap();
    }

    /// A PACKAGED H4CP ENTRY THAT ROTTED AFTER SEALING COSTS THE HOURS ITS
    /// DAMAGED RECORD REACHES, NOT THE RECORDING (task #192, residual HA-3).
    ///
    /// Lanes GZ and HA built per-record confinement inside
    /// `snapshot::CertifiedReplayCache`, and this door never reached it: both
    /// routes authenticated the container entry FIRST — the spooled route
    /// inside `spool_entry_to_file_transactionally_bounded`, the resident route
    /// inside `read_entry_bytes` — and a digest disagreement ended the open
    /// before one per-record digest was read. One rotted byte cost every hour.
    ///
    /// The fixture's cache has two records at checkpoint interval 2, so hour 10
    /// is the window's checkpoint and hour 11 its only delta. Rotting hour 11's
    /// record must cost hour 11 and leave hour 10 serving, byte-for-byte,
    /// through BOTH routes — which is also what proves the two routes still
    /// agree after the reordering.
    ///
    /// kills: restoring either container-first refusal. Put
    /// `spool_entry_to_file_transactionally_bounded` back and the spooled half
    /// fails at `open_recording_with_spooled_decoder`; put `read_entry_bytes`
    /// back and the resident half fails at `open_recording`. Deleting the
    /// four-outcome match instead does NOT fail this test — that arm is pinned
    /// by the recording being refused when the two accounts agree in the wrong
    /// direction, which no fixture here can build; it is stated at the code
    /// site the way the gate's identical arm is.
    #[test]
    fn a_rotted_packaged_cache_entry_confines_its_hours_instead_of_the_recording() {
        let root = temp_root("binary_replay_rotted_cache");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Omitted,
        );

        // Hour 11's record window comes from the container's OWN sealed H4IX,
        // not from a second copy of the encoder's arithmetic.
        let (cache_bytes, last_byte_of_hour_11) = {
            let container =
                snapshot::Container::open(&recording, &snapshot::ExpectedIdentity::default())
                    .unwrap();
            let index_bytes = container
                .read_entry_bytes(&format!(
                    "payload/binary-game/{}",
                    snapshot::HOUR_INDEX_FILE
                ))
                .unwrap();
            let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).unwrap();
            let row = &index.hours()[1];
            assert_eq!(row.hour_index(), 11, "position 1 serves hour 11");
            assert_eq!(index.hours()[0].hour_index(), 10);
            let offset = (row.cache_record_offset() + row.cache_record_len() - 1) as usize;
            let cache = container
                .read_entry_bytes(&format!(
                    "payload/binary-game/{}",
                    snapshot::CHECKPOINT_CACHE_FILE
                ))
                .unwrap();
            (cache, offset)
        };
        rot_sealed_cache_byte_in_recording(&recording, &cache_bytes, last_byte_of_hour_11);

        // The resident route.
        let resident = open_recording(&recording).unwrap();
        assert_eq!(resident.hours(), &[10, 11]);
        let frame = resident
            .frame(10)
            .expect("hour 10's record still hashes to the digest the seal published for it");
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.50));
        let refusal = resident.frame(11).unwrap_err().to_string();
        assert!(
            refusal.contains("game hour 11"),
            "the confined hour is refused by name: {refusal}"
        );

        // The spooled route reaches the same verdict over the same bytes.
        let spool_base = root.join("spool");
        fs::create_dir(&spool_base).unwrap();
        let spooled =
            open_recording_with_spooled_decoder(&recording, &spool_base, &UnusedSpoolDecoder)
                .unwrap();
        assert_eq!(spooled.hours(), resident.hours());
        assert_eq!(
            spooled.frame(10).unwrap().rest,
            resident.frame(10).unwrap().rest,
            "the served hour is byte-identical by route"
        );
        assert!(
            spooled.frame(11).is_err(),
            "the confined hour is confined by route too"
        );

        drop(spooled);
        drop(resident);
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// The composition is authority in both directions. A recording that
    /// declares the cache-only composition but ships a stream entry anyway is a
    /// rewritten recording and must fail closed rather than be quietly read.
    ///
    /// Production change that would make this fail: matching only on the entry
    /// count and ignoring the declared composition.
    #[test]
    fn declared_cache_only_recording_rejects_a_smuggled_stream_entry() {
        let root = temp_root("binary_replay_smuggled_stream");
        let recording = root.join("binary.battleplan");
        // Build the stream-on container, then rewrite ONLY the declaration.
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Packaged,
        );
        // Make the declaration fully coherent — every document says `omitted`
        // and none of them names a stream path — so the ONLY thing left
        // contradicting it is the entry that is still physically in the
        // container. That is the check this test is about.
        let repacked = repack_recording_with_manifest_mutation(&recording, |document| {
            document.insert(
                snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                json!(snapshot::RawStreamPackaging::Omitted.as_str()),
            );
            document.remove("stream");
            if let Some(entries) = document
                .get_mut("replay_entries")
                .and_then(Value::as_object_mut)
            {
                entries.remove("stream");
            }
        });
        fs::write(&recording, repacked).unwrap();

        let error = portable_test_fs_retry::with_read_guard(&recording, || {
            open_recording(&recording)
        })
        .expect("hold the recording open across the scanner's window")
            .err()
            .expect("a declared cache-only recording carrying a stream must fail closed");
        let text = error.to_string();
        assert!(
            text.contains("omitted") && text.contains(snapshot::COMPACT_STREAM_ROLE),
            "the refusal must name the contradiction, got {text}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// An unassigned composition spelling rejects at the decoder instead of
    /// falling back to a default.
    ///
    /// Production change that would make this fail: giving the composition
    /// reader a catch-all arm, or treating a bad value like a missing one.
    #[test]
    fn unassigned_composition_spelling_rejects_at_replay_admission() {
        let root = temp_root("binary_replay_bad_composition");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Packaged,
        );
        let repacked = repack_recording_with_manifest_mutation(&recording, |manifest| {
            manifest.insert(
                snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                json!("maybe"),
            );
        });
        fs::write(&recording, repacked).unwrap();

        let error = portable_test_fs_retry::with_read_guard(&recording, || {
            open_recording(&recording)
        })
        .expect("hold the recording open across the scanner's window")
            .err()
            .expect("an unassigned composition spelling must reject");
        let text = error.to_string();
        assert!(
            text.contains("unassigned") && text.contains("maybe"),
            "the rejection must name the unassigned value, got {text}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// `index.json` is the third document that states the composition, and it
    /// is cross-checked too. Without this, it would be the one copy a rewrite
    /// could change unnoticed while the manifest and timeline still agree.
    ///
    /// Production change that would make this fail: dropping the
    /// `declared_packaging` comparison from
    /// `validate_packaged_replay_cache_index_descriptor`.
    #[test]
    fn index_descriptor_composition_must_agree_with_the_recorder_authority() {
        let root = temp_root("binary_replay_index_composition");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture_with_composition(
            &recording,
            true,
            snapshot::RawStreamPackaging::Packaged,
        );
        // Rewrite ONLY index.json. The manifest and timeline still say
        // `packaged`, and the stream entry is genuinely present.
        let repacked = repack_recording_mutating_roles(&recording, &["index"], |index| {
            index.insert(
                snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                json!(snapshot::RawStreamPackaging::Omitted.as_str()),
            );
        });
        fs::write(&recording, repacked).unwrap();

        let error = portable_test_fs_retry::with_read_guard(&recording, || {
            open_recording(&recording)
        })
        .expect("hold the recording open across the scanner's window")
            .err()
            .expect("an index descriptor that disagrees about the composition must reject");
        let text = error.to_string();
        assert!(
            text.contains("index descriptor") && text.contains("omitted"),
            "the refusal must name the disagreeing document, got {text}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    /// Re-pack a fixture recording with its recorder manifest mutated, keeping
    /// every other entry, role and path identical. Used to perturb exactly one
    /// declaration and prove the reader notices.
    fn repack_recording_with_manifest_mutation(
        path: &Path,
        mutate: impl Fn(&mut serde_json::Map<String, Value>),
    ) -> Vec<u8> {
        repack_recording_mutating_roles(path, &["recorder_manifest", "timeline", "index"], mutate)
    }

    /// Re-pack a fixture recording, mutating only the named roles.
    fn repack_recording_mutating_roles(
        path: &Path,
        roles: &[&str],
        mutate: impl Fn(&mut serde_json::Map<String, Value>),
    ) -> Vec<u8> {
        let container =
            snapshot::Container::open(path, &snapshot::ExpectedIdentity::default()).unwrap();
        let game = container.game_id().to_string();
        let content = portable_content_manifest_fixture();
        let mut entries = Vec::new();
        for entry in container.entries() {
            let mut bytes = container.read_entry_bytes(&entry.path).unwrap();
            // All three documents state the composition and replay cross-checks
            // them, so a perturbation has to be applied to every document the
            // caller intends to keep consistent, or it trips an agreement check
            // instead of the check under test.
            if entry
                .role
                .as_deref()
                .is_some_and(|role| roles.contains(&role))
            {
                let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                mutate(value.as_object_mut().unwrap());
                bytes = serde_json::to_vec(&value).unwrap();
            }
            entries.push(snapshot::PackEntry {
                path: entry.path.clone(),
                bytes,
                role: Box::leak(
                    entry
                        .role
                        .clone()
                        .unwrap_or_default()
                        .into_boxed_str(),
                ),
                // The fixture carries no per-hour snapshot entries, so no entry
                // in it is hour-indexed.
                hour_index: None,
            });
        }
        snapshot::pack(
            &game,
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap()
    }

    #[test]
    fn packaged_binary_replay_rejects_an_incomplete_role_graph() {
        let root = temp_root("binary_replay_role");
        let recording = root.join("binary.battleplan");
        let _scanner_guard = write_portable_binary_replay_fixture(&recording, false);

        let error = match open_recording(&recording) {
            Ok(_) => panic!("binary replay with a mismatched checkpoint role must fail closed"),
            Err(error) => error,
        };
        assert!(error.to_string().contains(snapshot::CHECKPOINT_CACHE_ROLE));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    fn known_campaign_identity_value() -> Value {
        json!({
            "schema": "hoi4.campaign_identity.v1",
            "state": "known",
            "campaign_id": "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f",
            "game_unique_id": "a4b17fed-ed12-4361-9064-fb6576638110",
            "game_unique_seed": "1026046527",
            "source": "live_memory",
            "source_build_profile": "hoi4-a729",
            "observed_game_hour_raw": 60759359,
            "stream_header_sha256": "cc".repeat(32),
            "resolution_note": null,
            "witnesses": {
                "start_date_raw": null,
                "session": null,
                "scenario": null,
                "initial_human_country_tags": null,
                "modset_identity": null
            }
        })
    }

    #[test]
    fn recorder_campaign_identity_is_recomputed_before_replay_exposure() {
        let mut root = Map::new();
        root.insert(
            "campaign_identity".to_string(),
            known_campaign_identity_value(),
        );
        let identity = recorder_campaign_identity(&root, "test manifest")
            .unwrap()
            .expect("known identity must be exposed");
        assert_eq!(identity.state, "known");
        assert_eq!(
            identity.game_unique_id.as_deref(),
            Some("a4b17fed-ed12-4361-9064-fb6576638110")
        );
        assert_eq!(identity.game_unique_seed.as_deref(), Some("1026046527"));
        assert_eq!(
            identity.campaign_id.as_deref(),
            Some("b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f")
        );
        assert_eq!(identity.source.as_deref(), Some("live_memory"));
        assert_eq!(identity.resolution_note, None);
        assert_eq!(identity.witnesses.session, None);

        root.get_mut("campaign_identity")
            .unwrap()["game_unique_seed"] = json!("1026046528");
        assert!(recorder_campaign_identity(&root, "tampered manifest").is_err());
    }

    #[test]
    fn validated_known_manifest_selects_identity_v1_without_stream_sniffing() {
        let mut root = Map::new();
        root.insert(
            "campaign_identity".to_string(),
            known_campaign_identity_value(),
        );
        let known = recorder_campaign_identity(&root, "known mode")
            .unwrap()
            .expect("known identity");
        assert_eq!(
            prefold_stream_identity_mode_from_campaign(Some(&known)),
            snapshot::PrefoldStreamIdentityMode::IdentityV1
        );
        assert_eq!(
            prefold_stream_identity_mode_from_campaign(None),
            snapshot::PrefoldStreamIdentityMode::Legacy
        );
    }

    #[test]
    fn recorder_campaign_identity_rejects_extra_or_positive_non_known_authority() {
        let mut value = known_campaign_identity_value();
        value["unexpected"] = json!(true);
        let mut root = Map::new();
        root.insert("campaign_identity".to_string(), value);
        assert!(recorder_campaign_identity(&root, "extra key").is_err());

        let mut unavailable = known_campaign_identity_value();
        unavailable["state"] = json!("unavailable");
        unavailable["resolution_note"] = json!("live reader unavailable");
        unavailable["campaign_id"] = Value::Null;
        unavailable["game_unique_id"] = Value::Null;
        unavailable["game_unique_seed"] = Value::Null;
        unavailable["source"] = Value::Null;
        unavailable["observed_game_hour_raw"] = Value::Null;
        unavailable["stream_header_sha256"] = Value::Null;
        let mut root = Map::new();
        root.insert("campaign_identity".to_string(), unavailable.clone());
        let unavailable_identity = recorder_campaign_identity(&root, "unavailable")
            .unwrap()
            .expect("non-known metadata must remain inspectable");
        assert_eq!(
            unavailable_identity.resolution_note.as_deref(),
            Some("live reader unavailable")
        );

        unavailable["game_unique_seed"] = json!("1");
        root.insert("campaign_identity".to_string(), unavailable);
        assert!(recorder_campaign_identity(&root, "fabricated unavailable").is_err());
    }

    /// A packaged snapshot-flavor recording is bound to its stream entry just
    /// like the cache and raw-decoder routes are. That route serves straight
    /// from `StoreSource`, so without the binding on the common path it would
    /// accept a stream role whose identity contradicts the recorder's own
    /// capture receipt.
    ///
    /// Production change that would make this fail: letting the
    /// `(Packaged, 1)` arm of the composition reconciliation succeed without
    /// calling `validate_recorder_compact_stream_join`.
    #[test]
    fn packaged_snapshot_recording_binds_its_stream_entry_to_the_recorder_receipt() {
        let root = temp_root("snapshot_stream_binding");
        let recording = root.join("snapshot.battleplan");
        write_portable_recorder_fixture(&recording, "g", "SNAPSHOT", None);

        // The fixture is truthfully cache-only. Re-pack it as a PACKAGED
        // recording carrying a stream entry, but leave `stream_storage`
        // describing different bytes than the entry actually holds — exactly
        // the disagreement the join exists to catch.
        let stream_bytes = b"H4CRsnapshot-route-stream".to_vec();
        let container =
            snapshot::Container::open(&recording, &snapshot::ExpectedIdentity::default()).unwrap();
        let content = portable_content_manifest_fixture();
        let mut entries = Vec::new();
        for entry in container.entries() {
            let mut bytes = container.read_entry_bytes(&entry.path).unwrap();
            match entry.role.as_deref() {
                Some("recorder_manifest") => {
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    let object = value.as_object_mut().unwrap();
                    object.insert(
                        snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                        json!(snapshot::RawStreamPackaging::Packaged.as_str()),
                    );
                    object.insert(
                        "replay_entries".to_string(),
                        json!({
                            "stream": "collector_raw/collector_stream.bin",
                            "stream_role": snapshot::COMPACT_STREAM_ROLE,
                        }),
                    );
                    object.insert(
                        "stream_storage".to_string(),
                        json!({
                            // Deliberately not this entry's identity.
                            "decoded_bytes": stream_bytes.len() as u64 + 1,
                            "decoded_sha256": "a".repeat(64),
                        }),
                    );
                    bytes = serde_json::to_vec(&value).unwrap();
                }
                Some("timeline") | Some("index") => {
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    let object = value.as_object_mut().unwrap();
                    object.insert(
                        snapshot::RAW_STREAM_PACKAGING_FIELD.to_string(),
                        json!(snapshot::RawStreamPackaging::Packaged.as_str()),
                    );
                    if entry.role.as_deref() == Some("timeline") {
                        object.insert(
                            "stream".to_string(),
                            json!("collector_raw/collector_stream.bin"),
                        );
                    }
                    bytes = serde_json::to_vec(&value).unwrap();
                }
                _ => {}
            }
            entries.push(snapshot::PackEntry {
                path: entry.path.clone(),
                bytes,
                role: Box::leak(entry.role.clone().unwrap_or_default().into_boxed_str()),
                hour_index: if entry.role.as_deref() == Some("snapshot") {
                    Some(1)
                } else {
                    None
                },
            });
        }
        entries.push(snapshot::PackEntry {
            path: "payload/g/collector_raw/collector_stream.bin".to_string(),
            bytes: stream_bytes,
            role: snapshot::COMPACT_STREAM_ROLE,
            hour_index: None,
        });
        drop(container);
        let packed = snapshot::pack(
            "g",
            &entries,
            &snapshot::PackIdentity {
                schema_version: Some("1".into()),
                build_exe_sha256: Some(content.source_game.executable_sha256.clone()),
                build_pinned: true,
                content_manifest: Some(content),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        fs::write(&recording, packed).unwrap();

        let error = portable_test_fs_retry::with_read_guard(&recording, || {
            open_recording(&recording)
        })
        .expect("hold the recording open across the scanner's window")
            .err()
            .expect("a packaged snapshot recording must be bound to its stream entry");
        assert!(
            error.to_string().contains("stream_storage"),
            "the refusal must name the disagreeing authority, got {error}"
        );
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opened_recording_exposes_only_the_validated_recorder_campaign_identity() {
        let root = temp_root("campaign_identity_exposure");
        let recording = root.join("campaign.battleplan");
        write_portable_recorder_fixture(
            &recording,
            "g",
            "CAMPAIGN",
            Some(known_campaign_identity_value()),
        );

        let source = open_recording(&recording).unwrap();
        let identity = source
            .campaign_identity()
            .expect("valid recorder identity must survive source admission");
        assert_eq!(identity.state, "known");
        assert_eq!(identity.observed_game_hour_raw, Some(60759359));
        assert_eq!(
            identity.stream_header_sha256.as_deref(),
            Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
        );
        drop(source);
        let _ = fs::remove_dir_all(root);
    }

    fn prefolded_entry(hour: i64, schema: &str) -> snapshot::PrefoldedHourEntry {
        let bytes = serde_json::to_vec(&json!({
            "game_id": "prefolded-game",
            "schema_version": schema,
            "hour_index": hour,
            "date": "1936.1.1.12",
            "save_version": "a729",
            "countries": {"FRA": {"stability": 0.5}}
        }))
        .unwrap();
        snapshot::PrefoldedHourEntry::from_json(hour, &bytes).unwrap()
    }

    #[test]
    fn neutral_prefolded_hours_are_admitted_through_the_existing_frame_boundary() {
        let replay = snapshot::PrefoldedReplay::try_new(
            "prefolded-game",
            Value::String("1".into()),
            vec![prefolded_entry(10, "1"), prefolded_entry(12, "1")],
        )
        .unwrap();
        let source = open_prefolded_replay(replay).unwrap();

        assert_eq!(source.game_id(), "prefolded-game");
        assert_eq!(source.hours(), &[10, 12]);
        assert!(source.warnings().is_empty());
        let frame = source.frame(12).unwrap();
        assert_eq!(frame.hour_index, 12);
        assert_eq!(frame.date.as_deref(), Some("1936.1.1.12"));
        assert_eq!(frame.save_version.as_deref(), Some("a729"));
        assert_eq!(frame.schema_version, Some(Value::String("1".into())));
        assert_eq!(frame.rest["countries"]["FRA"]["stability"], json!(0.5));
        assert!(source.frame(11).is_err());
    }

    #[test]
    fn prefolded_admission_rejects_an_unsupported_schema_before_exposing_a_source() {
        let replay = snapshot::PrefoldedReplay::try_new(
            "prefolded-game",
            Value::String("2".into()),
            vec![prefolded_entry(10, "2")],
        )
        .unwrap();
        assert!(matches!(
            open_prefolded_replay(replay),
            Err(SourceError::Unsupported(_))
        ));
    }

    #[test]
    fn raw_h4cr_compact_package_remains_fail_closed_at_the_file_boundary() {
        let root = temp_root("raw_h4cr_stays_closed");
        let game = root.join("g");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.json"),
            br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#,
        )
        .unwrap();
        fs::write(
            game.join("timeline.json"),
            br#"{"format":"hoi4_compact_stream_timeline_v1","raw_stream_role":"collector_compact_stream"}"#,
        )
        .unwrap();
        fs::write(game.join("collector_stream.bin"), b"H4CR").unwrap();

        let error = match open_recording(&root) {
            Err(error) => error,
            Ok(_) => panic!("raw H4CR package must not enter replay_core directly"),
        };
        // R18-5: the compact-stream-only flavor is the TYPED subclass, so the
        // preflight consumer classifies it structurally instead of matching
        // "compact-stream-only production recording" in the message text.
        assert!(matches!(
            error,
            SourceError::UnsupportedCompactStreamOnly(_)
        ));
        let message = error.to_string();
        assert!(message.starts_with("unsupported recording: "), "{message}");
        assert!(message.contains("compact-stream-only"), "{message}");
        assert!(message.contains("collector_compact_stream"), "{message}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn empty_index_without_compact_evidence_stays_plain_unsupported() {
        // R18-5 boundary: only positive compact-stream evidence selects the
        // waivable typed subclass. A bare empty index must remain the plain
        // Unsupported flavor the preflight does NOT waive.
        let root = temp_root("empty_index_no_evidence");
        let game = root.join("g");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.json"),
            br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#,
        )
        .unwrap();
        let error = match open_recording(&root) {
            Err(error) => error,
            Ok(_) => panic!("an empty snapshot index must not open"),
        };
        assert!(matches!(error, SourceError::Unsupported(_)));
        assert!(error.to_string().contains("empty snapshot index"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn standalone_replay_manifest_has_no_extractor_dependency() {
        let manifest = include_str!("../Cargo.toml").replace('\\', "/");
        assert!(!manifest.contains("extractor/"), "{manifest}");
    }

    #[test]
    fn root_swap_full_lifecycle_stays_in_anchored_graph() {
        let parent = temp_root("root_swap");
        let recording = parent.join("store.battleplan");
        write_portable_recorder_fixture(&recording, "g", "ANCHORED", None);
        let moved = parent.join("anchored_object.battleplan");
        let source = open_recording_impl_with_post_anchor(&recording, None, None, None, |_| {
            // Windows gives the retained container handle delete-denial
            // semantics, so pathname replacement itself must fail. Unix
            // permits unlink/rename of an open object; there the replacement
            // succeeds and the retained handle must still supply ANCHORED.
            #[cfg(windows)]
            {
                fs::rename(&recording, &moved)
                    .expect_err("the retained container handle must deny pathname replacement");
                assert!(recording.is_file());
                assert!(!moved.exists());
            }
            #[cfg(not(windows))]
            {
                fs::rename(&recording, &moved).unwrap();
                write_portable_recorder_fixture(&recording, "g", "IMPOSTOR", None);
            }
        })
        .unwrap();

        let frame = source.frame(1).unwrap();
        assert_eq!(frame.rest["marker"], "ANCHORED");
        assert_eq!(frame.rest["session"]["active_dlcs"], json!([]));
        assert_eq!(frame.rest["session"]["localisation_language"], "english");
        assert_eq!(
            frame.rest["session"]["display_variant"]["physical_width"],
            1920
        );
        drop(source);
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn empty_index_evidence_after_root_swap_uses_anchored_timeline() {
        let parent = temp_root("empty_root_swap");
        let root = parent.join("store");
        let game = "g";
        let game_dir = root.join(game);
        fs::create_dir_all(&game_dir).unwrap();
        fs::write(
            game_dir.join("index.json"),
            serde_json::to_vec(&json!({"game_id": game, "snapshots": []})).unwrap(),
        )
        .unwrap();
        fs::write(
            game_dir.join("timeline.json"),
            br#"{"format":"hoi4_compact_stream_timeline_v1"}"#,
        )
        .unwrap();
        let moved = parent.join("anchored_object");
        let result = open_recording_impl_with_post_anchor(&root, None, None, None, |_| {
            fs::rename(&root, &moved).unwrap();
            let impostor = root.join(game);
            fs::create_dir_all(&impostor).unwrap();
            fs::write(
                impostor.join("index.json"),
                serde_json::to_vec(&json!({"game_id": game, "snapshots": []})).unwrap(),
            )
            .unwrap();
            fs::write(impostor.join("timeline.json"), br#"{"legacy":true}"#).unwrap();
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("empty index must be unsupported"),
        };
        let message = error.to_string();
        assert!(
            message.contains("hoi4_compact_stream_timeline_v1"),
            "{message}"
        );
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn directory_route_reads_each_metadata_object_once_and_uses_first_buffer() {
        let mut calls = BTreeMap::<String, usize>::new();
        let route = directory_recorder_metadata_route_with_reader(|relative| {
            let count = calls.entry(relative.to_string()).or_default();
            *count += 1;
            if *count > 1 {
                return Ok(Some(
                    br#"{"legacy":true,"second_read_must_not_bind":true}"#.to_vec(),
                ));
            }
            let bytes = match relative {
                "timeline.json" => {
                    br#"{"session_manifest":"recorder/native_session_manifest.json"}"#.as_slice()
                }
                "recorder/native_session_manifest.json" => br#"{"modset":{"resolved":true,"errors":[],"dlc":{"enabled":["dlc/a.dlc"]},"dlc_display_names":{"dlc/a.dlc":"DLC A"},"settings_binding":{"authority":"pre_launch_bracketed","launch_proven":true,"errors":[],"language":"l_english","display_variant":{"physical_width":2752,"physical_height":1152,"min_gui_width":1280,"min_gui_height":720,"gui_scale_f32_bits":1065353216,"refresh_rate_hz":60,"max_refresh_rate_hz":75,"fullscreen":false,"borderless":true,"display_mode":"borderless_fullscreen"},"attach_settings_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","seal_settings_sha256":null,"endpoint_semantics_equal":null}}}"#,
                other => panic!("unexpected metadata read {other:?}"),
            };
            Ok(Some(bytes.to_vec()))
        })
        .unwrap();

        let binding = route.binding.expect("first buffers must produce a binding");
        assert_eq!(binding.active_dlcs, vec!["DLC A"]);
        assert_eq!(binding.localisation_language, "l_english");
        assert_eq!(binding.display_variant.physical_width, 2752);
        assert_eq!(
            calls.get("timeline.json").copied(),
            Some(1),
            "timeline final object must be opened/read exactly once"
        );
        assert_eq!(
            calls.get("recorder/native_session_manifest.json").copied(),
            Some(1),
            "recorder-manifest final object must be opened/read exactly once"
        );
    }

    #[test]
    fn selected_game_directory_swap_before_frame_is_rejected_without_cross_object_mix() {
        let parent = temp_root("selected_game_swap");
        let root = parent.join("store");
        fs::create_dir_all(&root).unwrap();
        // Directory stores are a legacy compatibility surface. Recorder-owned
        // metadata is portable-container-only, so this object-swap test keeps
        // the directory fixture explicitly legacy while exercising the same
        // retained selected-game handle boundary.
        write_playable_store(&root, "g", false, "OBJECT_A");
        let source = open_recording(&root).unwrap();

        let original = root.join("g");
        let moved = root.join("game_object_a");
        fs::rename(&original, &moved).unwrap();
        write_playable_store(&root, "g", false, "OBJECT_B");

        let error = source
            .frame(1)
            .expect_err("a selected-game object swap must fail closed")
            .to_string();
        assert!(
            error.contains("identity") || error.contains("namespace"),
            "{error}"
        );
        drop(source);
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn timeline_to_manifest_game_parent_swap_is_rejected() {
        let parent = temp_root("timeline_manifest_parent_swap");
        let root = parent.join("store");
        fs::create_dir_all(&root).unwrap();
        write_playable_store(&root, "g", true, "OBJECT_A");
        let store = Store::open_directory(&root).unwrap();
        let mut swapped = false;

        let result = directory_recorder_metadata_route_with_reader(|relative| {
            let bytes = store
                .read_game_file_bytes("g", relative)
                .map_err(|error| SourceError::Open(error.to_string()))?;
            if relative == "timeline.json" && !swapped {
                fs::rename(root.join("g"), root.join("game_object_a")).unwrap();
                write_playable_store(&root, "g", true, "OBJECT_B");
                swapped = true;
            }
            Ok(bytes)
        });

        let error = match result {
            Err(error) => error.to_string(),
            Ok(_) => panic!("timeline and referenced manifest must share one game parent object"),
        };
        assert!(
            error.contains("identity") || error.contains("namespace"),
            "{error}"
        );
        drop(store);
        let _ = fs::remove_dir_all(parent);
    }

    #[derive(Default)]
    struct FilesystemAliases {
        modules: Vec<String>,
        files: Vec<String>,
        open_options: Vec<String>,
    }

    fn collect_aliases(compact: &str, markers: &[&str]) -> Vec<String> {
        let mut aliases = Vec::new();
        for marker in markers {
            let mut cursor = 0;
            while let Some(relative) = compact[cursor..].find(marker) {
                let start = cursor + relative + marker.len();
                let len = compact[start..]
                    .bytes()
                    .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                    .count();
                if len == 0 {
                    cursor = start;
                    continue;
                }
                let alias = &compact[start..start + len];
                if !aliases.iter().any(|existing| existing == alias) {
                    aliases.push(alias.to_string());
                }
                cursor = start + len;
            }
        }
        aliases
    }

    fn filesystem_aliases(source: &str) -> FilesystemAliases {
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        FilesystemAliases {
            modules: collect_aliases(
                &compact,
                &["usestd::fsas", "usestd::{fsas", "usestd::fs::{selfas"],
            ),
            files: collect_aliases(&compact, &["usestd::fs::Fileas", "usestd::fs::{Fileas"]),
            open_options: collect_aliases(
                &compact,
                &["usestd::fs::OpenOptionsas", "usestd::fs::{OpenOptionsas"],
            ),
        }
    }

    fn source_references_file_access(source: &str, aliases: &FilesystemAliases) -> bool {
        let compact = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        [
            "std::fs",
            "std::{fs",
            "File::",
            "OpenOptions",
            ".open(",
            "windows_sys::Win32::Storage::FileSystem",
            "windows_sys::Wdk::Storage::FileSystem",
            "::Storage::FileSystem::",
            "CreateFile(",
            "CreateFile2",
            "CreateFileA",
            "CreateFileW",
            "NtCreateFile",
            "ZwCreateFile",
            "NtOpenFile",
            "ZwOpenFile",
            "OpenFile(",
            "OpenFileById",
            "ReOpenFile",
        ]
        .iter()
        .any(|token| compact.contains(token))
            || aliases
                .modules
                .iter()
                .any(|alias| compact.contains(&format!("{alias}::")))
            || aliases
                .files
                .iter()
                .any(|alias| compact.contains(&format!("{alias}::")))
            || aliases
                .open_options
                .iter()
                .any(|alias| compact.contains(alias))
    }

    fn pathbuf_value_names(source: &str) -> Vec<String> {
        let mut names = Vec::new();
        for line in source.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("let ") else {
                continue;
            };
            let rest = rest.strip_prefix("mut ").unwrap_or(rest);
            let Some((name, value)) = rest.split_once('=') else {
                continue;
            };
            let name = name.trim().split(':').next().unwrap_or("").trim();
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                && (value.contains("PathBuf::")
                    || value.contains(".join(")
                    || value.contains(".to_path_buf("))
                && !names.iter().any(|existing| existing == name)
            {
                names.push(name.to_string());
            }
        }
        names
    }

    fn file_capability_import_lines(source: &str) -> Vec<bool> {
        let mut import_lines = vec![false; source.lines().count()];
        let mut in_import = false;
        for (index, line) in source.lines().enumerate() {
            let compact = line
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            if !in_import
                && (compact.contains("std::fs::{")
                    || compact.contains("std::{fs::{")
                    || (compact.contains("windows_sys::")
                        && compact.contains("::Storage::FileSystem::{")))
            {
                in_import = true;
            }
            if in_import {
                import_lines[index] = true;
                if compact.contains("};") {
                    in_import = false;
                }
            }
        }
        import_lines
    }

    const WINDOWS_OPEN_PRIMITIVES: &[&str] = &[
        "CreateFile",
        "CreateFile2",
        "CreateFileA",
        "CreateFileW",
        "CreateFile2FromAppW",
        "CreateFileFromAppW",
        "CreateFileTransactedA",
        "CreateFileTransactedW",
        "NtCreateFile",
        "ZwCreateFile",
        "NtOpenFile",
        "ZwOpenFile",
        "OpenFile",
        "OpenFileById",
        "ReOpenFile",
    ];

    fn windows_open_primitive_symbol(symbol: &str) -> bool {
        WINDOWS_OPEN_PRIMITIVES.contains(&symbol)
    }

    fn windows_open_primitive_candidate(line: &str) -> bool {
        WINDOWS_OPEN_PRIMITIVES.iter().any(|primitive| {
            line.match_indices(primitive).any(|(start, matched)| {
                let end = start + matched.len();
                let identifier_boundary = |byte: u8| !byte.is_ascii_alphanumeric() && byte != b'_';
                let starts_at_boundary = start == 0
                    || line
                        .as_bytes()
                        .get(start - 1)
                        .copied()
                        .is_some_and(identifier_boundary);
                let ends_at_boundary = end == line.len()
                    || line
                        .as_bytes()
                        .get(end)
                        .copied()
                        .is_some_and(identifier_boundary);
                if !starts_at_boundary || !ends_at_boundary {
                    return false;
                }
                let suffix = line[end..].trim_start();
                suffix.is_empty()
                    || suffix.starts_with('(')
                    || suffix.starts_with(',')
                    || suffix.starts_with(';')
                    || suffix.starts_with(')')
                    || suffix.starts_with('}')
                    || suffix.starts_with("as ")
            })
        })
    }

    fn link_name_open_primitive_candidate(line: &str) -> bool {
        let compact = line
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        let quote = char::from(34);
        let prefix = format!("#[link_name={quote}");
        let Some(attribute) = compact
            .find(&prefix)
            .map(|start| &compact[start + prefix.len()..])
        else {
            return false;
        };
        let Some(end) = attribute.find('"') else {
            return false;
        };
        windows_open_primitive_symbol(&attribute[..end])
    }

    fn ffi_open_declaration_candidate(line: &str) -> bool {
        link_name_open_primitive_candidate(line)
            || (line.contains("extern") && windows_open_primitive_candidate(line))
    }

    fn loader_pathname_candidate(
        line: &str,
        aliases: &FilesystemAliases,
        file_access_capable: bool,
        pathbuf_values: &[String],
    ) -> bool {
        let direct_file_access = [
            "std::fs",
            "std::{fs",
            "fs::",
            "File::",
            "OpenOptions",
            ".open(",
            ".is_file(",
            ".is_dir(",
            ".try_exists(",
            "windows_sys::Win32::Storage::FileSystem",
            "windows_sys::Wdk::Storage::FileSystem",
            "::Storage::FileSystem::",
        ]
        .iter()
        .any(|token| line.contains(token));
        let path_construction = file_access_capable
            && (["std::path", "Path::", "PathBuf::", ".join("]
                .iter()
                .any(|token| line.contains(token))
                || pathbuf_values
                    .iter()
                    .any(|name| line.contains(&format!("{name}.push("))));
        direct_file_access
            || windows_open_primitive_candidate(line)
            || ffi_open_declaration_candidate(line)
            || path_construction
            || aliases
                .modules
                .iter()
                .any(|alias| line.contains(&format!("{alias}::")))
            || aliases
                .files
                .iter()
                .any(|alias| line.contains(&format!("{alias}::open")))
            || aliases
                .open_options
                .iter()
                .any(|alias| line.contains(alias))
    }

    fn discover_rust_sources(label: &str, root: &Path) -> Result<Vec<(String, String)>, String> {
        fn walk(
            label: &str,
            root: &Path,
            directory: &Path,
            sources: &mut Vec<(String, String)>,
        ) -> Result<(), String> {
            let mut entries = fs::read_dir(directory)
                .map_err(|error| format!("discover {}: {error}", directory.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("enumerate {}: {error}", directory.display()))?;
            entries.sort_by_key(|entry| entry.path());
            for entry in entries {
                let file_type = entry.file_type().map_err(|error| {
                    format!("classify discovered {}: {error}", entry.path().display())
                })?;
                if file_type.is_symlink() {
                    return Err(format!(
                        "source discovery refuses symlink {}",
                        entry.path().display()
                    ));
                }
                if file_type.is_dir() {
                    walk(label, root, &entry.path(), sources)?;
                    continue;
                }
                if !file_type.is_file() {
                    return Err(format!(
                        "source discovery found unsupported object {}",
                        entry.path().display()
                    ));
                }
                if entry.path().extension().and_then(|value| value.to_str()) != Some("rs") {
                    continue;
                }
                let path = entry.path();
                let relative = path.strip_prefix(root).map_err(|_| {
                    format!(
                        "discovered source {} escaped root {}",
                        path.display(),
                        root.display()
                    )
                })?;
                let relative = relative
                    .to_str()
                    .ok_or_else(|| format!("source path is not Unicode: {}", path.display()))?
                    .replace(char::from(92), "/");
                let source = fs::read_to_string(&path).map_err(|error| {
                    format!("read discovered source {}: {error}", path.display())
                })?;
                sources.push((format!("{label}/{relative}"), source));
            }
            Ok(())
        }

        let root = fs::canonicalize(root)
            .map_err(|error| format!("canonicalize source root {}: {error}", root.display()))?;
        if !root.is_dir() {
            return Err(format!(
                "source root is not a directory: {}",
                root.display()
            ));
        }
        let mut sources = Vec::new();
        walk(label, &root, &root, &mut sources)?;
        if sources.is_empty() {
            return Err(format!(
                "source discovery found no Rust files under {label}"
            ));
        }
        Ok(sources)
    }

    fn classified_root_or_handle_site(module: &str, line: &str) -> bool {
        matches!(
            (module, line),
            ("replay_core/source.rs", "games.join(\", \")")
                // A message join over a static string table; no path is built.
                | (
                    "replay_core/source.rs",
                    "DECLARED_COVERAGE_KINDS.join(\", \")"
                )
                // Anchoring the recording root: ONE metadata read of the
                // caller-supplied path, then a typed classification of it.
                // No descendant pathname is constructed or opened here.
                | (
                    "replay_core/source.rs",
                    "let metadata = match fs::metadata(path) {"
                )
                | (
                    "replay_core/source.rs",
                    "let store = if metadata.is_file() {"
                )
                | ("replay_core/source.rs", "} else if metadata.is_dir() {")
                | ("snapshot/lib.rs", "if path.is_file() {")
                | (
                    "snapshot/container.rs",
                    "let data = std::fs::read(path.as_ref())?;"
                )
                | ("snapshot/dir_store/linux.rs", "use std::fs::File;")
                | ("snapshot/dir_store/linux.rs", "use std::path::Path;")
                | (
                    "snapshot/dir_store/linux.rs",
                    "let mut file = File::from(opened.fd);"
                )
                | ("snapshot/dir_store/windows.rs", "use std::fs::File;")
                | ("snapshot/dir_store/windows.rs", "use std::path::Path;")
                | (
                    "snapshot/dir_store/windows.rs",
                    "use windows_sys::Wdk::Storage::FileSystem::{"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "use windows_sys::Win32::Storage::FileSystem::{"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_OPEN_FOR_BACKUP_INTENT, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,"
                )
                | ("snapshot/dir_store/windows.rs", "};")
                | (
                    "snapshot/dir_store/windows.rs",
                    "CreateFileW, FileAttributeTagInfo, FileIdExtdDirectoryInfo, FileIdExtdDirectoryRestartInfo,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FileIdInfo, FileNormalizedNameInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_DIRECTORY,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_GENERIC_READ, FILE_ID_EXTD_DIR_INFO, FILE_ID_INFO, FILE_NAME_INFO, FILE_SHARE_DELETE,"
                )
                | (
                    "snapshot/dir_store/windows.rs",
                    "FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,"
                )
                | ("snapshot/dir_store/windows.rs", "CreateFileW(")
                | (
                    "snapshot/dir_store/windows.rs",
                    "let mut file = File::from(opened.handle);"
                )
                | ("snapshot/dir_store/windows.rs", "NtCreateFile(")
                | ("snapshot/stream_verify.rs", "use std::fs::File;")
                | ("snapshot/stream_verify.rs", "use std::path::Path;")
                | (
                    "snapshot/stream_verify.rs",
                    "File::open(path).map_err(|e| format!(\"open container {}: {e}\", path.display()))?;"
                )
        )
    }

    /// Exact inventory for the hardened portable-container root and output
    /// spool capabilities. The root reader lexically normalizes a caller path,
    /// rejects non-local Windows roots, retains every component no-follow, and
    /// denies delete rebinding. The spool sites create a private output file,
    /// authenticate its retained handle, and publish that exact object with a
    /// no-replace hard link. None is a manifest-selected descendant read.
    ///
    /// Keep every literal exact: a newly shaped open/join/push remains a gate
    /// failure until its ownership and handle-identity contract is reviewed.
    fn classified_snapshot_container_retained_io_site(module: &str, line: &str) -> bool {
        module == "snapshot/container.rs"
            && matches!(
                line,
                "use std::fs::{self, File, OpenOptions};"
                    | "use std::path::{Path, PathBuf};"
                    // String-only stitching alias canonicalization. This is
                    // Slice/String::join, not a filesystem Path::join.
                    | "let joined = components.join(\"/\");"
                    // Retained, no-follow container-root open.
                    | "use std::os::windows::fs::{MetadataExt, OpenOptionsExt};"
                    | "use std::path::{Component, Prefix};"
                    | "std::env::current_dir()?.join(path)"
                    | "let mut absolute = PathBuf::from(&prefix);"
                    | "absolute.push(\"\\\\\");"
                    | "absolute.push(name);"
                    | "let mut cursor = PathBuf::from(prefix);"
                    | "cursor.push(\"\\\\\");"
                    | "cursor.push(component.as_os_str());"
                    | "let directory = OpenOptions::new()"
                    | ".open(&cursor)?;"
                    | "if !metadata.is_dir()"
                    | "let file = OpenOptions::new()"
                    | ".open(&absolute)?;"
                    | "if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {"
                    | "File::open(path)"
                    // Authenticated transactional output-spool ownership and
                    // identity checks. These paths originate from the product
                    // spool owner, never from container manifest content.
                    | "use std::path::Component;"
                    | "std::env::current_dir()?.join(destination)"
                    | "let mut current = PathBuf::new();"
                    | "current.push(component.as_os_str());"
                    | "let metadata = fs::symlink_metadata(&current)?;"
                    | "if metadata.file_type().is_symlink() || !metadata.is_dir() {"
                    | "use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};"
                    | "let handle = OpenOptions::new()"
                    | ".open(&current)?;"
                    | "if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {"
                    | "let temp = guard.parent.join(temp_name);"
                    | "use std::os::windows::fs::OpenOptionsExt;"
                    | "OpenOptions::new()"
                    | ".open(path)"
                    | "use std::os::unix::fs::MetadataExt;"
                    | "if !metadata.is_file() {"
                    | "fs::hard_link(&temp, &destination).map_err(|error| {"
                    | "match fs::remove_file(&destination) {"
                    | "let _ = fs::remove_file(&temp);"
                    // Non-Windows arm of identity-gated spool cleanup. The
                    // pathname is the crate's OWN spool temporary or its own
                    // just-published destination, never a container-supplied
                    // name, and the unlink runs only after the pathname's file
                    // identity matched the retained handle. Windows disposes by
                    // handle instead and has no pathname site here.
                    | "match fs::remove_file(path) {"
            )
    }

    /// Exact capability census for CID1 rollover descriptor discovery. These
    /// sites only identify/read the caller-selected package-set descriptor or
    /// perform bounded, portable path validation; they do not open a member
    /// selected by an unauthenticated descendant pathname. Keep the list exact
    /// so any new discovery primitive or descriptor read fails closed here.
    fn classified_rollover_descriptor_discovery_site(module: &str, line: &str) -> bool {
        module == "replay_core/rollover_package_set.rs"
            && matches!(
                line,
                "use std::fs;"
                    | "use std::path::{Component, Path, PathBuf};"
                    | "let descriptor_path = if path.is_dir() {"
                    | "let metadata = fs::symlink_metadata(path).map_err(|error| {"
                    | "if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {"
                    | "path.join(ROLLOVER_PACKAGE_SET_FILE)"
                    | ".unwrap_or_else(|| Path::new(\".\"));"
                    | "let parent_metadata = fs::symlink_metadata(descriptor_parent).map_err(|error| {"
                    | "if !parent_metadata.file_type().is_dir() || parent_metadata.file_type().is_symlink() {"
                    | "if source_descriptor != Path::new(ROLLOVER_SOURCE_SET_FILE) {"
                    | "|| Path::new(&source_stream.relative_path) != expected_relative"
                    | "let discovered_descriptor = path.is_dir() && path.join(ROLLOVER_PACKAGE_SET_FILE).is_file();"
                    | "let path = PathBuf::from(value);"
                    | "let metadata = fs::symlink_metadata(path)"
                    | "if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {"
                    | "let file = fs::File::open(path)"
            )
    }

    /// Exact capability census for descriptor-authorized rollover members.
    /// The descriptor supplies strict relative paths and expected digests;
    /// resolution rejects links/escapes, and bytes are consumed only from the
    /// retained file/container handle whose receipt is then checked. This is
    /// intentionally line-exact rather than a module-wide filesystem waiver.
    fn classified_rollover_retained_handle_site(module: &str, line: &str) -> bool {
        module == "replay_core/rollover_package_set.rs"
            && matches!(
                line,
                "let root = fs::canonicalize(root)"
                    | "let candidate = root.join(relative);"
                    | "let metadata = fs::symlink_metadata(&candidate)"
                    | "if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {"
                    | "let canonical = fs::canonicalize(&candidate)"
                    | "let mut file = fs::File::open(&canonical).map_err(|error| {"
                    // Whole-file SHA-256 probe over the already-resolved,
                    // descriptor-authorized package path, run only to classify
                    // an open that has ALREADY failed (integrity vs structure).
                    // It admits nothing: the failed open stays failed.
                    | "let file = fs::File::open(path).ok()?;"
            )
    }

    fn classified_non_loader_site(module: &str, line: &str) -> bool {
        match module {
            // Test-only module: `world.rs` includes it under `#[cfg(test)]`, so
            // none of this compiles into a shipping build. The audit walks
            // `src/` by filename and cannot see that, so the four lines of the
            // XMAP19 joint-certification fixture read are named here. They open
            // ONE committed evidence file by a fixed name under the repo's own
            // `replay_interface/fixtures`, never a path from a recording, and
            // nothing about a replay input is selected from them.
            "replay_core/world_xmap19_tests.rs" => matches!(
                line,
                "let path = std::path::PathBuf::from(env!(\"CARGO_MANIFEST_DIR\"))"
                    | ".join(\"../../fixtures\")"
                    | ".join(\"xmap19_weather_visual_lifecycle_joint_certification.json\");"
                    | "let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {"
            ),
            "snapshot/bin/container_codec_bench.rs" => matches!(
                line,
                "use std::fs::File;"
                    | "use std::path::PathBuf;"
                    | ".map(PathBuf::from)"
                    | "let meta = std::fs::metadata(&path).map_err(|err| format!(\"stat {}: {err}\", path.display()))?;"
                    | "let file = File::open(path).map_err(|err| format!(\"open {}: {err}\", path.display()))?;"
            ),
            "snapshot/bin/pack_replay.rs" => matches!(
                line,
                "use std::fs::{self, OpenOptions};"
                    | "use std::io::{Read, Seek, SeekFrom, Write};"
                    | "use std::path::{Path, PathBuf};"
                    | "(Some(a), Some(b), Some(c)) => (PathBuf::from(a), b.clone(), PathBuf::from(c)),"
                    | "let gdir = root.join(&game);"
                    | "let index_path = gdir.join(\"index.json\");"
                    | "let index_bytes = match fs::read(&index_path) {"
                    | "let p = gdir.join(file);"
                    | "let bytes = match fs::read(&p) {"
                    | "let tpath = gdir.join(\"timeline.json\");"
                    | "if let Ok(tb) = fs::read(&tpath) {"
                    | "fs::create_dir_all(parent)"
                    | "let canonical_parent = fs::canonicalize(parent.unwrap_or_else(|| Path::new(\".\")))"
                    | "let anchored_out = canonical_parent.join(file_name);"
                    | "let published = fs::read(&anchored_out)"
                    // Output-only atomic publication helpers. `out` is the
                    // caller-selected destination; the temporary child is
                    // created with create_new beneath its canonicalized
                    // parent and is never used to select replay input.
                    | "fn create_unique_temp(out: &Path) -> Result<(PathBuf, fs::File), String> {"
                    | "let parent = out.parent().unwrap_or_else(|| Path::new(\".\"));"
                    | "let temp = parent.join(format!(\".{name}.pack-tmp-{nonce}.{attempt}\"));"
                    | "match OpenOptions::new()"
                    | ".open(&temp)"
                    | "match fs::symlink_metadata(out) {"
                    | "use std::os::windows::fs::MetadataExt;"
                    | "let cleanup = fs::remove_file(&temp);"
                    | "fs::File::open(out.parent().unwrap_or_else(|| Path::new(\".\")))?.sync_all()"
                    | "fs::rename(temp, out)"
                    | "let gdir = Path::new(root).join(game);"
                    | "let orig = gdir.join(&s.file);"
                    | "let orig_bytes = match fs::read(&orig) {"
                    | "Some(p) => PathBuf::from(p),"
            ),
            "snapshot/pack.rs" => matches!(
                line,
                "use std::fs;"
                    | "use std::path::Path;"
                    | "return Ok(fs::metadata(entry.file_path)?.len());"
                    | "let mut file = fs::File::open(path)?;"
            ),
            "replay_core/bin/make_demo_recording.rs" => matches!(
                line,
                "use std::path::{Path, PathBuf};"
                    | "bundle_dir = Some(PathBuf::from(args.get(i).ok_or_else(|| USAGE.to_string())?));"
                    | "out_path = Some(PathBuf::from(args.get(i).ok_or_else(|| USAGE.to_string())?));"
                    | "std::fs::write(&out_path, packed).map_err(|e| format!(\"write {}: {e}\", out_path.display()))?;"
                    | "let path = bundle_dir.join(\"state_owner_1936.json\");"
                    | "std::fs::read_to_string(&path).map_err(|e| format!(\"read {}: {e}\", path.display()))?;"
                    | "let path = bundle_dir.join(\"province_to_state.json\");"
            ),
            "replay_core/bin/replay_dump.rs" => matches!(
                line,
                "coverage.domains.join(\", \")" | ".join(\",\")" | ".join(\", \")"
            ),
            "replay_core/world.rs" => matches!(
                line,
                "path[..=i].join(\".\"),"
                    | "path.join(\".\"),"
                    | "let ectx = format!(\"{ctx}.{}[{i}]\", path.join(\".\"));"
                    | "let rctx = format!(\"{ctx}.{}[{i}]\", path.join(\".\"));"
                    | "path.join(\".\")"
            ),
            "replay_core/lib.rs" => matches!(
                line,
                // Exact module re-export. The lexical F4 lint sees the `fs`
                // suffix in `defs`; this line exposes no filesystem
                // capability and performs no pathname operation.
                "pub use static_focus_defs::{"
            ),
            _ => false,
        }
    }

    /// Exact production file-capability census for the replay-owned prefold
    /// spool. These are not caller-selected descendant reads: replay_core
    /// creates a private, identity-marked child beneath its product-owned
    /// parent, rejects links/replacements/extra entries before every read, and
    /// removes only that authenticated child. Keep this list exact so any new
    /// filesystem operation still fails the F4 gate pending review.
    /// The certified checkpoint cache reads from a handle its CALLER opened.
    ///
    /// `replay_cache.rs` imports `File` and seeks within one, but it builds no
    /// pathname and opens nothing: `CertifiedCacheFile::new` is handed an
    /// already-open handle by the product spool site below. These two lines are
    /// exactly that fact, and the module cannot acquire a descendant-pathname
    /// read without this list changing.
    fn classified_certified_cache_handle_site(module: &str, line: &str) -> bool {
        module == "snapshot/replay_cache.rs"
            && matches!(
                line,
                "use std::fs::File;" | "let cache = CertifiedCacheFile::new(cache)?;"
            )
    }

    fn classified_private_prefold_spool_site(module: &str, line: &str) -> bool {
        module == "replay_core/source.rs"
            && matches!(
                line,
                "use std::fs;"
                    | "use std::path::{Path, PathBuf};"
                    | "let raw_path = owner.root().join(\"source.h4cr\");"
                    // The packaged checkpoint cache, spooled into this same
                    // private root and reopened read-only for bounded seeks.
                    | "let spooled = owner.root().join(\"checkpoints.h4cp\");"
                    // Task #192 moved the CREATE of that file here from
                    // `Container::spool_entry_to_file_transactionally_bounded`,
                    // because the container's own verdict on the entry can no
                    // longer be what decides whether the bytes are written —
                    // the sealed H4IX judges them record by record afterwards.
                    // The capability is unchanged in reach: the destination is
                    // one fixed name inside the identity-marked private root
                    // this call just created, and `create_new(true)` refuses to
                    // replace anything.
                    | "let mut sink = fs::OpenOptions::new()"
                    | ".open(&spooled)"
                    | "fn open_private_prefold_spool_file(spool_path: &Path) -> Result<fs::File, SourceError> {"
                    | "let spool_file = fs::OpenOptions::new()"
                    | ".open(spool_path)"
                    | "let file = fs::OpenOptions::new()"
                    | ".open(&raw_path)"
                    | "let meta = fs::symlink_metadata(base).map_err(|error| {"
                    | "if !meta.file_type().is_dir() || meta.file_type().is_symlink() {"
                    | "let canonical_base = fs::canonicalize(base).map_err(|error| {"
                    | "let root = canonical_base.join(format!("
                    | "match fs::create_dir(&root) {"
                    | "let canonical_root = match fs::canonicalize(&root) {"
                    | "let _ = fs::remove_dir(&root);"
                    | "let _ = fs::remove_dir_all(&root);"
                    | "let marker_path = canonical_root.join(PREFOLD_SPOOL_MARKER);"
                    | "let mut marker = fs::OpenOptions::new()"
                    | ".open(&marker_path)"
                    | "let snapshot_dir = canonical_root.join(\"snapshots\");"
                    | "fs::create_dir(&snapshot_dir).map_err(|error| {"
                    | "if fs::canonicalize(&root).ok().as_deref()"
                    | "let Ok(meta) = fs::symlink_metadata(&self.root) else {"
                    | "if fs::canonicalize(&self.root).ok().as_deref() != Some(self.canonical_root.as_path()) {"
                    | "let Ok(snapshot_meta) = fs::symlink_metadata(&self.snapshot_dir) else {"
                    | "if !snapshot_meta.file_type().is_dir() || snapshot_meta.file_type().is_symlink() {"
                    | "if fs::canonicalize(&self.snapshot_dir).ok().as_deref() != Some(self.snapshot_dir.as_path())"
                    | "let marker = self.root.join(PREFOLD_SPOOL_MARKER);"
                    | "let Ok(marker_meta) = fs::symlink_metadata(&marker) else {"
                    | "if !marker_meta.file_type().is_file() || marker_meta.file_type().is_symlink() {"
                    | "if fs::read_to_string(marker).ok().as_deref() != Some(self.marker_value.as_str()) {"
                    | "let Ok(root_entries) = fs::read_dir(&self.root) else {"
                    | "let Ok(meta) = fs::symlink_metadata(entry.path()) else {"
                    | "if !meta.file_type().is_file() || meta.file_type().is_symlink() {"
                    | "let Ok(canonical) = fs::canonicalize(entry.path()) else {"
                    | "let Ok(snapshot_entries) = fs::read_dir(&self.snapshot_dir) else {"
                    | "let path = self.root.join(\"source.h4cr\");"
                    | "let meta = fs::symlink_metadata(&path).map_err(|error| {"
                    | "let canonical = fs::canonicalize(&path).map_err(|error| {"
                    | "if !meta.file_type().is_file()"
                    | "fs::remove_file(&path).map_err(|error| {"
                    | "let _ = fs::remove_dir_all(&self.root);"
                    | "if fs::canonicalize(snapshot_dir).ok().as_deref() != Some(owner.snapshot_dir()) {"
                    | "let path = snapshot_dir.join(entry.file_name());"
                    | "let path_meta = fs::symlink_metadata(&path).map_err(|error| {"
                    | "if !path_meta.file_type().is_file()"
                    | "let file = fs::File::open(&canonical).map_err(|error| {"
            )
    }

    fn cfg_test_item_end(source: &str, mut cursor: usize) -> Result<usize, String> {
        #[derive(Clone, Copy)]
        enum State {
            Code,
            LineComment,
            BlockComment(usize),
            String(bool),
            RawString(usize),
        }

        fn raw_string_start(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
            let mut cursor = match (bytes.get(index), bytes.get(index + 1)) {
                (Some(b'r'), _) => index + 1,
                (Some(b'b'), Some(b'r')) => index + 2,
                _ => return None,
            };
            let mut hashes = 0;
            while bytes.get(cursor) == Some(&b'#') {
                hashes += 1;
                cursor += 1;
            }
            (bytes.get(cursor) == Some(&b'"')).then_some((cursor + 1, hashes))
        }

        let bytes = source.as_bytes();
        let mut state = State::Code;
        let mut depth = 0usize;
        let mut opened = false;
        while cursor < bytes.len() {
            match state {
                State::Code => {
                    if bytes[cursor..].starts_with(b"//") {
                        state = State::LineComment;
                        cursor += 2;
                        continue;
                    }
                    if bytes[cursor..].starts_with(b"/*") {
                        state = State::BlockComment(1);
                        cursor += 2;
                        continue;
                    }
                    if let Some((next, hashes)) = raw_string_start(bytes, cursor) {
                        state = State::RawString(hashes);
                        cursor = next;
                        continue;
                    }
                    if bytes[cursor] == b'"' {
                        state = State::String(false);
                        cursor += 1;
                        continue;
                    }
                    if bytes[cursor] == b'\'' {
                        let mut end = cursor + 1;
                        if bytes.get(end) == Some(&b'\\') {
                            end += 2;
                        } else if let Some(character) = source[end..].chars().next() {
                            end += character.len_utf8();
                        }
                        if bytes.get(end) == Some(&b'\'') {
                            cursor = end + 1;
                            continue;
                        }
                    }
                    match bytes[cursor] {
                        b'{' => {
                            opened = true;
                            depth += 1;
                        }
                        b'}' if opened => {
                            depth = depth.checked_sub(1).ok_or_else(|| {
                                "cfg(test) item brace depth underflow".to_string()
                            })?;
                            if depth == 0 {
                                return Ok(cursor + 1);
                            }
                        }
                        b';' if !opened => return Ok(cursor + 1),
                        _ => {}
                    }
                    cursor += 1;
                }
                State::LineComment => {
                    if bytes[cursor] == b'\n' {
                        state = State::Code;
                    }
                    cursor += 1;
                }
                State::BlockComment(comment_depth) => {
                    if bytes[cursor..].starts_with(b"/*") {
                        state = State::BlockComment(comment_depth + 1);
                        cursor += 2;
                    } else if bytes[cursor..].starts_with(b"*/") {
                        if comment_depth == 1 {
                            state = State::Code;
                        } else {
                            state = State::BlockComment(comment_depth - 1);
                        }
                        cursor += 2;
                    } else {
                        cursor += 1;
                    }
                }
                State::String(escaped) => {
                    if escaped {
                        state = State::String(false);
                    } else if bytes[cursor] == b'\\' {
                        state = State::String(true);
                    } else if bytes[cursor] == b'"' {
                        state = State::Code;
                    }
                    cursor += 1;
                }
                State::RawString(hashes) => {
                    if bytes[cursor] == b'"'
                        && bytes
                            .get(cursor + 1..cursor + 1 + hashes)
                            .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
                    {
                        state = State::Code;
                        cursor += 1 + hashes;
                    } else {
                        cursor += 1;
                    }
                }
            }
        }
        Err("cfg(test) item did not have a complete body".into())
    }

    fn mask_cfg_test_items(source: &str) -> Result<String, String> {
        const ATTRIBUTE: &str = "#[cfg(test)]";
        let mut masked = source.as_bytes().to_vec();
        let mut cursor = 0;
        while let Some(relative) = source[cursor..].find(ATTRIBUTE) {
            let start = cursor + relative;
            let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
            let line_end = source[start..]
                .find('\n')
                .map_or(source.len(), |index| start + index);
            if source[line_start..line_end].trim() != ATTRIBUTE {
                cursor = start + ATTRIBUTE.len();
                continue;
            }
            let end = cfg_test_item_end(source, start + ATTRIBUTE.len())?;
            for byte in &mut masked[start..end] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
            cursor = end;
        }
        String::from_utf8(masked).map_err(|_| "masked Rust source stopped being UTF-8".into())
    }

    /// F4 dossier/dispatch scope ruling (2026-07-14): this is a
    /// development-time, best-effort honest-mistake lint, not a runtime
    /// security boundary. It cheaply recognizes direct APIs and explicit
    /// link_name FFI declarations. Renamed FFI beyond that check,
    /// macro/proc-macro expansion, and generated/include! code remain known,
    /// ACCEPTED blind spots; the runtime boundary is the handle-relative
    /// loader implementation and its identity/namespace checks.
    fn loader_pathname_audit(module: &str, source: &str) -> Result<(usize, Vec<String>), String> {
        let source = mask_cfg_test_items(source)?;
        let aliases = filesystem_aliases(&source);
        let file_access_capable = source_references_file_access(&source, &aliases);
        let pathbuf_values = pathbuf_value_names(&source);
        let import_lines = file_capability_import_lines(&source);
        let mut classified = 0;
        let mut candidates = 0;
        let mut violations = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let line = line.trim();
            if line.starts_with("//")
                || (!import_lines[index]
                    && !loader_pathname_candidate(
                        line,
                        &aliases,
                        file_access_capable,
                        &pathbuf_values,
                    ))
            {
                continue;
            }
            candidates += 1;
            if classified_root_or_handle_site(module, line)
                || classified_snapshot_container_retained_io_site(module, line)
                || classified_rollover_descriptor_discovery_site(module, line)
                || classified_rollover_retained_handle_site(module, line)
                || classified_non_loader_site(module, line)
                || classified_private_prefold_spool_site(module, line)
                || classified_certified_cache_handle_site(module, line)
            {
                classified += 1;
            } else {
                violations.push(format!("{module}:{}: {line}", index + 1));
            }
        }
        if file_access_capable && candidates == 0 {
            violations.push(format!(
                "{module}: file-access capability yielded zero recognized source sites"
            ));
        }
        Ok((classified, violations))
    }

    fn loader_pathname_violations(module: &str, source: &str) -> Vec<String> {
        loader_pathname_audit(module, source)
            .expect("pathname audit source must be structurally unambiguous")
            .1
    }

    #[test]
    fn f4_gate_catches_planted_file_and_open_options_descendant_opens() {
        let planted = r#"
use std::fs::{File, OpenOptions};
use std::{fs as aliased_fs, path::Path};
use std::fs as disk;
fn bad(root: &std::path::Path) {
    let _ = File::open(root.join("timeline.json"));
    let _ = OpenOptions::new().read(true).open(root.join("index.json"));
    let _ = aliased_fs::read(root.join("snapshot.json"));
    let _ = disk::read(root.join("aliased.json"));
}
"#;
        let violations = loader_pathname_violations("snapshot/container.rs", planted);
        assert!(
            violations.iter().any(|site| site.contains("File::open")),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("OpenOptions")),
            "{violations:?}"
        );
        assert!(
            violations
                .iter()
                .any(|site| site.contains("aliased_fs::read")),
            "{violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("disk::read")),
            "{violations:?}"
        );
    }

    #[test]
    fn f4_gate_catches_create_file2_and_pathbuf_push_open_sites() {
        for primitive in [
            "CreateFile2",
            "CreateFileA",
            "CreateFileW",
            "NtCreateFile",
            "ZwCreateFile",
            "NtOpenFile",
            "ZwOpenFile",
            "OpenFileById",
            "ReOpenFile",
        ] {
            let planted = format!(
                "use windows_sys::Win32::Storage::FileSystem::{primitive};\n\
                 fn bad() {{ let _ = {primitive}(0); }}\n"
            );
            let violations =
                loader_pathname_violations(&format!("fixture/{primitive}.rs"), &planted);
            assert!(
                violations.iter().any(|site| site.contains(primitive)),
                "{primitive} must never be an invisible zero-site module: {violations:?}"
            );
        }

        let split_primitive = r#"
use windows_sys::Win32::Storage::FileSystem::CreateFileW;
fn bad() {
    let _ = CreateFileW
        (0);
}
"#;
        let violations =
            loader_pathname_violations("fixture/split_create_file.rs", split_primitive);
        assert!(
            violations
                .iter()
                .any(|site| site.contains("let _ = CreateFileW")),
            "a whitespace-split primitive name must still be classified: {violations:?}"
        );

        let future_windows_api = r#"
use windows_sys::Win32::Storage::FileSystem::{
    FutureFileOpenPrimitive,
};
"#;
        let violations =
            loader_pathname_violations("fixture/future_windows_api.rs", future_windows_api);
        assert!(
            violations
                .iter()
                .any(|site| site.contains("FutureFileOpenPrimitive")),
            "an unknown windows-sys FileSystem capability must fail closed: {violations:?}"
        );

        let pathbuf_push_open = r#"
use std::fs::File;
use std::path::PathBuf;
fn bad(root: &std::path::Path) {
    let mut child = PathBuf::from(root);
    child.push("descendant.json");
    let _ = File::open(child);
}
"#;
        let violations =
            loader_pathname_violations("fixture/pathbuf_push_open.rs", pathbuf_push_open);
        assert!(
            violations.iter().any(|site| site.contains(".push(")),
            "PathBuf::push in a file-open-capable module must be classified: {violations:?}"
        );
        assert!(
            violations.iter().any(|site| site.contains("File::open")),
            "the paired file open must be classified: {violations:?}"
        );
    }

    #[test]
    fn f4_gate_recognizes_link_name_file_open_ffi_declaration() {
        let planted = r#"
#[link(name = "kernel32")]
extern "system" {
    #[link_name = "CreateFileW"]
    fn renamed_open();
}
"#;
        let violations = loader_pathname_violations("fixture/ffi_alias.rs", planted);
        assert!(
            violations.iter().any(|site| site.contains("#[link_name")),
            "the FFI capability declaration itself must be a recognized site: {violations:?}"
        );
    }

    #[test]
    fn f4_gate_discovers_new_module_and_scans_production_after_test_item() {
        let root = temp_root("f4_closed_world");
        fs::write(root.join("existing.rs"), "fn existing() {}\n").unwrap();
        fs::write(
            root.join("newly_added_loader_helper.rs"),
            r#"
use std::fs::File;
fn bad(root: &std::path::Path) {
    let _ = File::open(root.join("new-module.json"));
}
"#,
        )
        .unwrap();
        let discovered = discover_rust_sources("fixture", &root).unwrap();
        let (_, new_module) = discovered
            .iter()
            .find(|(module, _)| module == "fixture/newly_added_loader_helper.rs")
            .expect("new module is discovered without a hand-maintained module list");
        let new_module_violations =
            loader_pathname_violations("fixture/newly_added_loader_helper.rs", new_module);
        assert!(
            new_module_violations
                .iter()
                .any(|site| site.contains("File::open")),
            "{new_module_violations:?}"
        );

        // The two `File::open` calls below are PLANTED SOURCE TEXT that the
        // pathname audit is asked to scan; they are not calls this test makes.
        // They must stay spelled exactly as the audit recognises them.
        let after_test = r#"
#[cfg(test)]
mod tests {
    fn test_only(root: &std::path::Path) {
        let _ = std::fs::File::open(root.join("test-only.json"));
    }
}

fn production(root: &std::path::Path) {
    let _ = std::fs::File::open(root.join("after-test.json"));
}
"#;
        let after_test_violations = loader_pathname_violations("fixture/after_test.rs", after_test);
        assert!(
            after_test_violations
                .iter()
                .any(|site| site.contains("after-test.json")),
            "{after_test_violations:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_sources_have_no_descendant_pathname_read() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut modules = discover_rust_sources("replay_core", &manifest_dir.join("src"))
            .expect("discover every replay_core production source");
        // The REAL container/loader implementation replay_core links lives in
        // portable_replay/native/snapshot (Cargo.toml: snapshot = portable).
        // Its command-line drivers (src/bin/pack_replay.rs and friends) live
        // there too, so this one neutral root is the whole audited surface.
        //
        // This scan used to add `extractor/native/snapshot/src` as a second
        // root, because the bins were on the extractor side. That was a
        // filesystem reach across the replay/extractor boundary that the cargo
        // dependency gate cannot see
        // (`replay_interface/native/game_ui/tests/replay_dependency_boundary.rs`
        // reads `cargo metadata`, not paths a test opens). The bins moved to
        // the neutral layer instead of the reach being kept: the module labels
        // are unchanged (`snapshot/bin/pack_replay.rs`, ...), so every
        // classification below still names exactly the same site it did before.
        // What remains on the extractor side is `pub use
        // portable_replay_format::*;` and three `[[bin]]` declarations, neither
        // of which can open a file.
        modules.extend(
            discover_rust_sources(
                "snapshot",
                &manifest_dir.join("../../../portable_replay/native/snapshot/src"),
            )
            .expect("discover every portable snapshot production source"),
        );
        let discovered = modules.len();
        let mut classified = 0;
        let mut violations = Vec::new();
        for (module, source) in modules {
            let (module_classified, mut module_violations) =
                loader_pathname_audit(&module, &source).unwrap_or_else(|error| {
                    panic!("pathname audit could not classify {module}: {error}")
                });
            classified += module_classified;
            violations.append(&mut module_violations);
        }
        assert!(
            violations.is_empty(),
            "unclassified or unrecognized file-access sites across {discovered} discovered modules ({classified} classified): {violations:#?}"
        );
    }
}
