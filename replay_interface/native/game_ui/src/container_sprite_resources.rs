//! Viewer-owned ordinary file-texture registrations. This cache holds one
//! registration per exact native path until its session is dropped. Drawing
//! borrows that registration; it does not register again on each raster pass.
//! 1402c6f30 uses byte-exact keys; 1423f17e0 preserves first-entry options;
//! 1423f4160 removes a name only after the last owning reference is released.
//! Static cache/lifetime hypotheses and their remaining corroboration gaps are
//! tracked in extractor/native/runs/arrow_native_ui_20260908/CANDIDATE_RESOURCE_LIFECYCLES.json.
//! Fixture checks do not establish native lifetime/order or production initialization.

use crate::assets::AssetResolver;
use crate::container_sprite_geometry::{
    OrdinarySingleMipTextureSource, OrdinarySpriteTextureLoadOptions,
    OrdinaryTextureSourceRejection,
};
use crate::dds::RgbaImage;
use crate::gfx::SpriteDefinition;
use crate::paradox_text::{Entry, Scalar, Value};
use replay_core::UsedContentKey;
use snapshot::sha256::Sha256;
use std::collections::BTreeMap;
use std::sync::Arc;

#[path = "container_sprite_resources_gpu.rs"]
mod gpu;
pub(crate) use gpu::OrdinaryTextureGpuRejection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrdinaryTextureRole {
    Base,
    MaskingTexture,
    AnimationMask(usize),
    AnimationTexture(usize),
}

/// Each reached role supplies its own native filename and admitted winner.
/// Roles do not partition the native path cache: aliases across roles share
/// first-registration options when the path bytes are identical.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OrdinaryTextureRequest<'a> {
    pub role: OrdinaryTextureRole,
    pub native_path: &'a str,
    pub content: &'a UsedContentKey,
}

/// A filename assignment and its portable winner, owned beyond the source AST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnedOrdinaryTextureRequest {
    pub role: OrdinaryTextureRole,
    pub native_path: String,
    pub content: UsedContentKey,
    /// Zero-based ordinal among all fields of this sprite, including repeats.
    pub field_ordinal: usize,
}

impl OwnedOrdinaryTextureRequest {
    pub(crate) fn borrowed(&self) -> OrdinaryTextureRequest<'_> {
        OrdinaryTextureRequest {
            role: self.role,
            native_path: &self.native_path,
            content: &self.content,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OrdinarySpriteTextureField {
    Absent,
    Empty { field_ordinal: usize },
    File(OwnedOrdinaryTextureRequest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OrdinarySpriteRequestCause {
    UnauthenticatedSource,
    SourceReadUnavailable,
    SourcePayloadIdentityMismatch,
    UnsupportedSourceEncoding,
    SourceParseRecovery,
    SourceDefinitionMismatch,
    AmbiguousSourceDefinition,
    UnsupportedDefinitionKindOrCase,
    UnsupportedFieldCaseOrQuotedKey,
    UnsupportedFieldType,
    UnsupportedMacroReference,
    UnsupportedBooleanValue,
    NativeUnreadableString,
    UnsupportedSourceTokenSpelling,
    Resource(OrdinaryTextureResourceCause),
}

/// CSpriteType+1d3. No Default: an absent authored value copies a separate
/// constructor global, whose execution phase must be established by its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrdinarySpriteLoadSchedule {
    Eager,
    LegacyDeferred,
}

impl OrdinarySpriteLoadSchedule {
    pub(crate) fn from_native_legacy_lazy_load(value: bool) -> Self {
        if value {
            Self::LegacyDeferred
        } else {
            Self::Eager
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrdinarySpriteRequestRejection {
    pub source_file: String,
    pub sprite_source_ordinal: Option<[usize; 2]>,
    pub field_ordinal: Option<usize>,
    pub role: Option<OrdinaryTextureRole>,
    pub cause: OrdinarySpriteRequestCause,
}

/// Only the two filename fields owned by the ordinary sprite reader. Animation
/// record filenames remain owned by their existing reader and registry roles.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OrdinarySpriteTextureRequests {
    pub source: UsedContentKey,
    /// [root entry ordinal, member ordinal within that spriteTypes block].
    pub sprite_source_ordinal: [usize; 2],
    pub base: OrdinarySpriteTextureField,
    pub masking_texture: OrdinarySpriteTextureField,
    /// Explicit successfully assigned token 655, if present. Absence does not
    /// establish the constructor-global value at this type's creation time.
    pub load_schedule: Option<OrdinarySpriteLoadSchedule>,
    pub load_schedule_field_ordinal: Option<usize>,
    /// The base type loader's +40 category. Actual constructors set zero;
    /// successfully extracted loadType assigns 1/2/3 or zero in source order.
    pub initialization_category: i32,
    pub initialization_category_field_ordinal: Option<usize>,
    // Exact admitted strings, integers and ordered AST. No floating values or
    // lossy serialization participate in this retained definition identity.
    definition: SpriteDefinition,
}

// SpriteDefinition and its raw Value contain only structural strings/integers.
// Their PartialEq is reflexive; retain Eq for transactional owner comparisons.
impl Eq for OrdinarySpriteTextureRequests {}

impl OrdinarySpriteTextureRequests {
    /// Draw-time identity check. This neither reparses the definition nor
    /// resolves a cached texture again; a different same-file row cannot use
    /// this type bundle merely because its source-file key is equal.
    pub(crate) fn matches_definition(
        &self,
        resolver: &AssetResolver,
        sprite: &SpriteDefinition,
    ) -> bool {
        resolver.has_portable_content_admission()
            && resolver.admitted_content_key(&sprite.source_file) == Some(&self.source)
            && sprite == &self.definition
    }

    /// Bind the supplied already-selected definition to its actual admitted
    /// source. This does not select a sprite-name winner or establish a load.
    pub(crate) fn from_definition(
        resolver: &AssetResolver,
        sprite: &SpriteDefinition,
    ) -> Result<Self, OrdinarySpriteRequestRejection> {
        let source_error = |cause| OrdinarySpriteRequestRejection {
            source_file: sprite.source_file.clone(),
            sprite_source_ordinal: None,
            field_ordinal: None,
            role: None,
            cause,
        };
        if !resolver.has_portable_content_admission() {
            return Err(source_error(
                OrdinarySpriteRequestCause::UnauthenticatedSource,
            ));
        }
        let source = resolver
            .admitted_content_key(&sprite.source_file)
            .filter(|key| key.validate().is_ok())
            .ok_or_else(|| source_error(OrdinarySpriteRequestCause::UnauthenticatedSource))?;
        let file = resolver
            .resolve_file(&sprite.source_file)
            .map_err(|_| source_error(OrdinarySpriteRequestCause::SourceReadUnavailable))?;
        if resolver.admitted_content_key(&file.rel_path) != Some(source) {
            return Err(source_error(
                OrdinarySpriteRequestCause::UnauthenticatedSource,
            ));
        }
        let bytes = resolver
            .read_file(&file)
            .map_err(|_| source_error(OrdinarySpriteRequestCause::SourceReadUnavailable))?;
        if bytes.len() as u64 != source.payload_bytes
            || Sha256::hex(&bytes) != source.payload_sha256
        {
            return Err(source_error(
                OrdinarySpriteRequestCause::SourcePayloadIdentityMismatch,
            ));
        }
        if !bytes.is_ascii() || bytes.contains(&0) {
            return Err(source_error(
                OrdinarySpriteRequestCause::UnsupportedSourceEncoding,
            ));
        }
        let text = std::str::from_utf8(&bytes).expect("ASCII source");
        let document = crate::paradox_text::parse_str(text);
        if !document.recoveries.is_empty() {
            return Err(source_error(
                OrdinarySpriteRequestCause::SourceParseRecovery,
            ));
        }
        let mut matched = None;
        for (root_ordinal, container) in document.root.fields().iter().enumerate() {
            if !container.key_matches("spriteTypes") {
                continue;
            }
            for (member_ordinal, entry) in container.value.fields().iter().enumerate() {
                if entry.key.as_deref() != Some(sprite.kind.as_str()) || entry.value != sprite.raw {
                    continue;
                }
                if crate::gfx::parse_sprite(&sprite.kind, &sprite.source_file, &entry.value)
                    .as_ref()
                    != Some(sprite)
                {
                    return Err(source_error(
                        OrdinarySpriteRequestCause::SourceDefinitionMismatch,
                    ));
                }
                if matched.is_some() {
                    return Err(source_error(
                        OrdinarySpriteRequestCause::AmbiguousSourceDefinition,
                    ));
                }
                if container.key_is_quoted
                    || container.key.as_deref() != Some("spriteTypes")
                    || entry.key_is_quoted
                    || sprite.kind != "spriteType"
                {
                    return Err(source_error(
                        OrdinarySpriteRequestCause::UnsupportedDefinitionKindOrCase,
                    ));
                }
                matched = Some([root_ordinal, member_ordinal]);
            }
        }
        let ordinal = matched
            .ok_or_else(|| source_error(OrdinarySpriteRequestCause::SourceDefinitionMismatch))?;
        let mut base = None;
        let mut masking = None;
        let mut initialization_category = 0;
        let mut initialization_category_field_ordinal = None;
        let mut load_schedule = None;
        let mut load_schedule_field_ordinal = None;
        for (field_ordinal, field) in sprite.raw.fields().iter().enumerate() {
            if field.key_matches("loadType") {
                let fail = |cause| OrdinarySpriteRequestRejection {
                    source_file: sprite.source_file.clone(),
                    sprite_source_ordinal: Some(ordinal),
                    field_ordinal: Some(field_ordinal),
                    role: None,
                    cause,
                };
                if field.key_is_quoted || field.key.as_deref() != Some("loadType") {
                    return Err(fail(
                        OrdinarySpriteRequestCause::UnsupportedFieldCaseOrQuotedKey,
                    ));
                }
                let value = field
                    .value
                    .as_scalar()
                    .ok_or_else(|| fail(OrdinarySpriteRequestCause::UnsupportedFieldType))?;
                if matches!(&field.value, Value::Scalar(Scalar::Bare(value)) if value.starts_with('@'))
                {
                    return Err(fail(OrdinarySpriteRequestCause::UnsupportedMacroReference));
                }
                if !plain_source_assignment(text, field, "loadType", value) {
                    return Err(fail(
                        OrdinarySpriteRequestCause::UnsupportedSourceTokenSpelling,
                    ));
                }
                initialization_category = match value {
                    "FRONTEND" => 1,
                    "BACKEND" => 2,
                    "INGAME" => 3,
                    _ => 0,
                };
                initialization_category_field_ordinal = Some(field_ordinal);
                continue;
            }
            if field.key_matches("legacy_lazy_load") {
                let fail = |cause| OrdinarySpriteRequestRejection {
                    source_file: sprite.source_file.clone(),
                    sprite_source_ordinal: Some(ordinal),
                    field_ordinal: Some(field_ordinal),
                    role: None,
                    cause,
                };
                if field.key_is_quoted || field.key.as_deref() != Some("legacy_lazy_load") {
                    return Err(fail(
                        OrdinarySpriteRequestCause::UnsupportedFieldCaseOrQuotedKey,
                    ));
                }
                let value = field
                    .value
                    .as_scalar()
                    .ok_or_else(|| fail(OrdinarySpriteRequestCause::UnsupportedFieldType))?;
                if matches!(&field.value, Value::Scalar(Scalar::Bare(value)) if value.starts_with('@'))
                {
                    return Err(fail(OrdinarySpriteRequestCause::UnsupportedMacroReference));
                }
                if !plain_source_assignment(text, field, "legacy_lazy_load", value) {
                    return Err(fail(
                        OrdinarySpriteRequestCause::UnsupportedSourceTokenSpelling,
                    ));
                }
                // 1424b06e0 compares exactly yes\0 or no\0. Failure leaves
                // the destination unchanged, but 1424abca0 reports an error.
                // Do not silently repair that parser error with a later field.
                load_schedule = Some(match value {
                    "yes" => OrdinarySpriteLoadSchedule::LegacyDeferred,
                    "no" => OrdinarySpriteLoadSchedule::Eager,
                    _ => return Err(fail(OrdinarySpriteRequestCause::UnsupportedBooleanValue)),
                });
                load_schedule_field_ordinal = Some(field_ordinal);
                continue;
            }
            let (spelling, role, target) = if field.key_matches("textureFile") {
                ("textureFile", OrdinaryTextureRole::Base, &mut base)
            } else if field.key_matches("masking_texture") {
                (
                    "masking_texture",
                    OrdinaryTextureRole::MaskingTexture,
                    &mut masking,
                )
            } else {
                continue;
            };
            let fail = |cause| OrdinarySpriteRequestRejection {
                source_file: sprite.source_file.clone(),
                sprite_source_ordinal: Some(ordinal),
                field_ordinal: Some(field_ordinal),
                role: Some(role),
                cause,
            };
            if field.key_is_quoted || field.key.as_deref() != Some(spelling) {
                return Err(fail(
                    OrdinarySpriteRequestCause::UnsupportedFieldCaseOrQuotedKey,
                ));
            }
            let literal = match &field.value {
                Value::Scalar(Scalar::Bare(value)) if value.starts_with('@') => {
                    return Err(fail(OrdinarySpriteRequestCause::UnsupportedMacroReference));
                }
                Value::Scalar(Scalar::Bare(value) | Scalar::Quoted(value)) => value,
                Value::Object(_) => {
                    return Err(fail(OrdinarySpriteRequestCause::UnsupportedFieldType))
                }
            };
            if !plain_source_assignment(text, field, spelling, literal) {
                return Err(fail(
                    OrdinarySpriteRequestCause::UnsupportedSourceTokenSpelling,
                ));
            }
            // 1424abb50 flags LF and overwrites with "Unreadable String". Do
            // not pretend the earlier successful assignment survives it. This
            // follows the source check: a parser-decoded escape is unproved.
            if literal.contains('\n') {
                return Err(fail(OrdinarySpriteRequestCause::NativeUnreadableString));
            }
            *target = Some((field_ordinal, literal.as_str()));
        }
        let bind = |field: Option<(usize, &str)>,
                    role|
         -> Result<OrdinarySpriteTextureField, OrdinarySpriteRequestRejection> {
            let Some((field_ordinal, path)) = field else {
                return Ok(OrdinarySpriteTextureField::Absent);
            };
            if path.is_empty() {
                return Ok(OrdinarySpriteTextureField::Empty { field_ordinal });
            }
            let fail = |cause| OrdinarySpriteRequestRejection {
                source_file: sprite.source_file.clone(),
                sprite_source_ordinal: Some(ordinal),
                field_ordinal: Some(field_ordinal),
                role: Some(role),
                cause: OrdinarySpriteRequestCause::Resource(cause),
            };
            validate_path(path).map_err(fail)?;
            let file = resolver
                .resolve_texture_file(path)
                .map_err(|_| fail(OrdinaryTextureResourceCause::ResolveUnavailable))?;
            let content = resolver
                .admitted_content_key(&file.rel_path)
                .filter(|key| key.validate().is_ok())
                .ok_or_else(|| fail(OrdinaryTextureResourceCause::UnauthenticatedContent))?;
            Ok(OrdinarySpriteTextureField::File(
                OwnedOrdinaryTextureRequest {
                    role,
                    native_path: path.to_owned(),
                    content: content.clone(),
                    field_ordinal,
                },
            ))
        };
        Ok(Self {
            source: source.clone(),
            sprite_source_ordinal: ordinal,
            base: bind(base, OrdinaryTextureRole::Base)?,
            masking_texture: bind(masking, OrdinaryTextureRole::MaskingTexture)?,
            load_schedule,
            load_schedule_field_ordinal,
            initialization_category,
            initialization_category_field_ordinal,
            definition: sprite.clone(),
        })
    }
}

/// The portable AST stores unescaped values. Check this particular source
/// assignment so a removed backslash cannot manufacture a native path. This
/// narrow branch accepts direct ASCII scalars with whitespace around '=';
/// comments between key/value and escaped tokens remain named unsupported.
fn plain_source_assignment(text: &str, entry: &Entry, key: &str, literal: &str) -> bool {
    let Some(line) = text.split('\n').nth(entry.line.saturating_sub(1)) else {
        return false;
    };
    let Some(column) = entry.column.checked_sub(1) else {
        return false;
    };
    if column > line.len() {
        return false;
    }
    let line_start = text
        .split_inclusive('\n')
        .take(entry.line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    let Some(rest) = text
        .get(line_start + column..)
        .and_then(|s| s.strip_prefix(key))
    else {
        return false;
    };
    let Some(rest) = rest
        .trim_start_matches(|c: char| c.is_ascii_whitespace())
        .strip_prefix('=')
    else {
        return false;
    };
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
    match &entry.value {
        Value::Scalar(Scalar::Quoted(_)) => rest
            .strip_prefix('"')
            .and_then(|s| s.split_once('"'))
            .is_some_and(|(raw, _)| raw == literal && !raw.contains(['\\', '\r'])),
        Value::Scalar(Scalar::Bare(_)) => {
            let end = rest
                .find(|c: char| c.is_ascii_whitespace() || matches!(c, '{' | '}' | '=' | '#'))
                .unwrap_or(rest.len());
            &rest[..end] == literal && !literal.contains(['\\', '"'])
        }
        Value::Object(_) => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OrdinaryTextureResourceCause {
    UnsupportedRegistrationPath,
    UnauthenticatedResolver,
    UnauthenticatedContent,
    ResolveUnavailable,
    WrongResolvedWinner,
    ReadUnavailable,
    PayloadIdentityMismatch,
    ChangedSessionNamespace,
    ChangedCachedContent,
    MissingLoadSetting,
    Source(OrdinaryTextureSourceRejection),
    DecodeUnavailable,
    DecodedDimensionsDisagree,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrdinaryTextureResourceRejection {
    pub role: OrdinaryTextureRole,
    pub native_path: String,
    pub cause: OrdinaryTextureResourceCause,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegisteredOrdinaryTexture {
    content: UsedContentKey,
    encoded: Arc<[u8]>,
    image: Arc<RgbaImage>,
    effective_options: OrdinarySpriteTextureLoadOptions,
    force_pow2_at_load: bool,
}

impl RegisteredOrdinaryTexture {
    pub(crate) fn content(&self) -> &UsedContentKey {
        &self.content
    }
    pub(crate) fn encoded(&self) -> &[u8] {
        &self.encoded
    }
    pub(crate) fn image(&self) -> &Arc<RgbaImage> {
        &self.image
    }
    pub(crate) fn dimensions(&self) -> [u32; 2] {
        [self.image.width as u32, self.image.height as u32]
    }
    pub(crate) fn effective_options(&self) -> OrdinarySpriteTextureLoadOptions {
        self.effective_options
    }
    /// Allocation history only. Current draw-time UV normalization still uses
    /// the current explicit profile setting, even when this resource is cached.
    pub(crate) fn force_pow2_at_load(&self) -> bool {
        self.force_pow2_at_load
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Registration {
    content: UsedContentKey,
    effective_options: OrdinarySpriteTextureLoadOptions,
    force_pow2_at_load: bool,
    result: Result<Arc<RegisteredOrdinaryTexture>, OrdinaryTextureResourceCause>,
}

/// Sole owner of this viewer resource-cache lifetime, separate from image-only
/// UiAssetCache entries. Clone this with a candidate when registration changes
/// must roll back with presentation. Clones share immutable bytes and pixels.
/// Never seed this cache from an unqualified DecodedTexture/RgbaImage alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrdinarySpriteTextureRegistry {
    namespace: Option<String>,
    entries: BTreeMap<String, Registration>,
}

/// The physical authored type and the admission under which it was created.
/// The retained row separately keeps the full source key and exact definition.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct OrdinarySpriteTypeKey {
    manifest_namespace: String,
    source_path: String,
    type_ordinal: [usize; 2],
}

impl OrdinarySpriteTypeKey {
    fn from_requests(manifest_namespace: &str, requests: &OrdinarySpriteTextureRequests) -> Self {
        Self {
            manifest_namespace: manifest_namespace.into(),
            source_path: requests.source.normalized_game_relative_path.clone(),
            type_ordinal: requests.sprite_source_ordinal,
        }
    }
}

/// Capability for one reached native-library type-initialization operation.
/// Only the forthcoming library owner may establish its production minting
/// law. There is deliberately no production constructor or guessed widget-to-
/// library conversion here. The operation sequence is retained provenance,
/// never substituted for native type order by this resource table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrdinarySpriteTypeInitializationOperation {
    manifest_namespace: String,
    source: UsedContentKey,
    type_ordinal: [usize; 2],
    category: i32,
    operation_sequence: u64,
    records: crate::container_animation_records::AdmittedContainerAnimations,
}

impl OrdinarySpriteTypeInitializationOperation {
    #[cfg(test)]
    pub(crate) fn fixture(
        assets: &AssetResolver,
        requests: &OrdinarySpriteTextureRequests,
        records: &crate::container_animation_records::AdmittedContainerAnimations,
        operation_sequence: u64,
    ) -> Self {
        Self {
            manifest_namespace: ViewerSpriteResources::admitted_type_manifest(assets)
                .unwrap()
                .into(),
            source: requests.source.clone(),
            type_ordinal: requests.sprite_source_ordinal,
            category: requests.initialization_category,
            operation_sequence,
            records: records.clone(),
        }
    }

    fn matches_requests(
        &self,
        manifest_namespace: &str,
        requests: &OrdinarySpriteTextureRequests,
        records: &crate::container_animation_records::AdmittedContainerAnimations,
    ) -> bool {
        self.manifest_namespace == manifest_namespace
            && self.source == requests.source
            && self.type_ordinal == requests.sprite_source_ordinal
            && self.category == requests.initialization_category
            && self.records == *records
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RetainedOrdinarySpriteType {
    first_operation: OrdinarySpriteTypeInitializationOperation,
    records: crate::container_animation_records::AdmittedContainerAnimations,
    constructor_schedule: Option<OrdinarySpriteLoadSchedule>,
    resources: crate::container_sprite_resource_schedule::InitializedOrdinarySpriteTypeResources,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OrdinarySpriteTypeOwnerRejection {
    InitializationAuthorityUnavailable,
    UnauthenticatedDefinition,
    ChangedAdmission,
    OperationIdentityMismatch,
    ConflictingTypeIdentity,
    AmbiguousTypeIdentity,
    Schedule(crate::container_sprite_resource_schedule::OrdinarySpriteScheduleRejection),
}

/// One viewer-owned resource lifetime, independent of conference widgets,
/// viewport reconstruction and current profile. Its owner stages this value
/// with the full viewer state and commits it only after successful Present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerSpriteResources {
    pub(crate) ordinary: OrdinarySpriteTextureRegistry,
    type_namespace: Option<String>,
    ordinary_types: BTreeMap<OrdinarySpriteTypeKey, RetainedOrdinarySpriteType>,
}

impl ViewerSpriteResources {
    pub fn new_viewer_session() -> Self {
        Self {
            ordinary: OrdinarySpriteTextureRegistry::new_viewer_session(),
            type_namespace: None,
            ordinary_types: BTreeMap::new(),
        }
    }
    fn admitted_type_manifest(
        assets: &AssetResolver,
    ) -> Result<&str, OrdinarySpriteTypeOwnerRejection> {
        assets
            .icon_content_lease()
            .ok()
            .and_then(|lease| lease.manifest_digest_sha256())
            .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or(OrdinarySpriteTypeOwnerRejection::UnauthenticatedDefinition)
    }

    /// Raster lookup is read-only: no parsing, registration, type construction
    /// or assignment of a guessed native initialization flag occurs here.
    pub(crate) fn type_resources_for(
        &self,
        assets: &AssetResolver,
        sprite: &SpriteDefinition,
    ) -> Result<
        &crate::container_sprite_resource_schedule::InitializedOrdinarySpriteTypeResources,
        String,
    > {
        let key = self
            .matched_type_key(assets, sprite)
            .map_err(|error| format!("{error:?}"))?;
        Ok(&self.ordinary_types[&key].resources)
    }

    fn matched_type_key(
        &self,
        assets: &AssetResolver,
        sprite: &SpriteDefinition,
    ) -> Result<OrdinarySpriteTypeKey, OrdinarySpriteTypeOwnerRejection> {
        use OrdinarySpriteTypeOwnerRejection as Reject;
        let source = assets
            .admitted_content_key(&sprite.source_file)
            .filter(|key| assets.has_portable_content_admission() && key.validate().is_ok())
            .ok_or(Reject::UnauthenticatedDefinition)?;
        let manifest_namespace = Self::admitted_type_manifest(assets)?;
        if self
            .type_namespace
            .as_ref()
            .is_some_and(|namespace| namespace != manifest_namespace)
        {
            return Err(Reject::ChangedAdmission);
        }
        let mut matches = self.ordinary_types.iter().filter(|(_, entry)| {
            entry.resources.requests.source == *source
                && entry.resources.requests.matches_definition(assets, sprite)
        });
        let Some((key, _)) = matches.next() else {
            return Err(Reject::InitializationAuthorityUnavailable);
        };
        if matches.next().is_some() {
            return Err(Reject::AmbiguousTypeIdentity);
        }
        Ok(key.clone())
    }

    /// A library owner that has actually reached initialization supplies the
    /// capability and all original operands. The returned whole-owner clone
    /// remains staged until its caller commits the presentation transaction.
    /// There is no public insertion path for an arbitrary completed bundle.
    pub(crate) fn stage_reached_type_initialization(
        &self,
        assets: &AssetResolver,
        operation: &OrdinarySpriteTypeInitializationOperation,
        requests: &OrdinarySpriteTextureRequests,
        records: &crate::container_animation_records::AdmittedContainerAnimations,
        constructor_schedule: Option<OrdinarySpriteLoadSchedule>,
        force_pow2_at_new_load: Option<bool>,
        animation_programs_retain_vector: Option<bool>,
    ) -> Result<Self, OrdinarySpriteTypeOwnerRejection> {
        use crate::container_sprite_resource_schedule::InitializedOrdinarySpriteTypeResources;
        use OrdinarySpriteTypeOwnerRejection as Reject;
        if !requests.matches_definition(assets, &requests.definition)
            || requests.source.validate().is_err()
        {
            return Err(Reject::UnauthenticatedDefinition);
        }
        let namespace = Self::admitted_type_manifest(assets)?;
        if !operation.matches_requests(namespace, requests, records)
            || records.source_identity != requests.source
        {
            return Err(Reject::OperationIdentityMismatch);
        }
        if self
            .type_namespace
            .as_ref()
            .is_some_and(|known| known != namespace)
            || self
                .ordinary
                .namespace
                .as_ref()
                .is_some_and(|known| known != &requests.source.effective_vfs_manifest_sha256)
        {
            return Err(Reject::ChangedAdmission);
        }
        let key = OrdinarySpriteTypeKey::from_requests(namespace, requests);
        if let Some(retained) = self.ordinary_types.get(&key) {
            // Mirror the initialized +44 arm without rerunning resource
            // effects. A later operation or profile cannot reset that type.
            // Conflicting same-physical-row inputs remain explicit instead of
            // acquiring an order-dependent first-wins definition.
            if retained.resources.requests != *requests
                || retained.records != *records
                || retained.constructor_schedule != constructor_schedule
            {
                return Err(Reject::ConflictingTypeIdentity);
            }
            return Ok(self.clone());
        }
        let (ordinary, resources) = InitializedOrdinarySpriteTypeResources::stage_type_initialization(
            &self.ordinary,
            assets,
            requests,
            records,
            constructor_schedule,
            force_pow2_at_new_load,
            animation_programs_retain_vector,
        )
        .map_err(Reject::Schedule)?;
        let mut next = self.clone();
        next.ordinary = ordinary;
        next.type_namespace = Some(namespace.into());
        next.ordinary_types.insert(
            key,
            RetainedOrdinarySpriteType {
                first_operation: operation.clone(),
                records: records.clone(),
                constructor_schedule,
                resources,
            },
        );
        Ok(next)
    }

    /// Only the actual reached CIcon position operation calls this getter.
    /// It cannot manufacture a type if library initialization is unavailable.
    /// On failure neither the type nor any newly requested texture survives.
    pub(crate) fn stage_icon_size_getter(
        &mut self,
        assets: &AssetResolver,
        sprite: &SpriteDefinition,
        force_pow2: Option<bool>,
    ) -> Result<crate::container_sprite_resource_schedule::InitializedOrdinarySpriteTypeResources, String>
    {
        let key = self
            .matched_type_key(assets, sprite)
            .map_err(|error| format!("{error:?}"))?;
        let retained = &self.ordinary_types[&key];
        let (ordinary, resources) = retained
            .resources
            .stage_size_getter(&self.ordinary, assets, force_pow2)
            .map_err(|error| format!("{:?}", OrdinarySpriteTypeOwnerRejection::Schedule(error)))?;
        // Both fallible operations have completed before either field changes.
        self.ordinary = ordinary;
        self.ordinary_types
            .get_mut(&key)
            .expect("matched retained type")
            .resources = resources.clone();
        Ok(resources)
    }
}


impl OrdinarySpriteTextureRegistry {
    /// An explicitly new viewer-owned resource lifetime. Do not call per draw
    /// or clear on a profile change: existing allocations retain load history.
    pub(crate) fn new_viewer_session() -> Self {
        Self {
            namespace: None,
            entries: BTreeMap::new(),
        }
    }

    /// Includes refused entries, so diagnostics can identify the retained
    /// allocation inputs rather than repeat a later request's desired inputs.
    pub(crate) fn retained_load_inputs(
        &self,
        native_path: &str,
    ) -> Option<(OrdinarySpriteTextureLoadOptions, bool)> {
        self.entries
            .get(native_path)
            .map(|entry| (entry.effective_options, entry.force_pow2_at_load))
    }

    pub(crate) fn get_or_register_ordinary(
        &mut self,
        resolver: &AssetResolver,
        request: OrdinaryTextureRequest<'_>,
        force_pow2_at_new_load: Option<bool>,
    ) -> Result<Arc<RegisteredOrdinaryTexture>, OrdinaryTextureResourceRejection> {
        self.get_or_register_file_request(
            resolver,
            request,
            OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration(),
            force_pow2_at_new_load,
        )
    }

    /// Other witnessed file-loader requesters may establish the first entry
    /// through this same owner. A cache hit ignores new requested options and
    /// new allocation policy; it returns the retained registration/result.
    pub(crate) fn get_or_register_file_request(
        &mut self,
        resolver: &AssetResolver,
        request: OrdinaryTextureRequest<'_>,
        requested_options: OrdinarySpriteTextureLoadOptions,
        force_pow2_at_new_load: Option<bool>,
    ) -> Result<Arc<RegisteredOrdinaryTexture>, OrdinaryTextureResourceRejection> {
        let refuse = |cause| rejection(request, cause);
        validate_path(request.native_path).map_err(refuse)?;
        if !resolver.has_portable_content_admission() {
            return Err(refuse(
                OrdinaryTextureResourceCause::UnauthenticatedResolver,
            ));
        }
        if request.content.validate().is_err()
            || resolver.admitted_content_key(&request.content.normalized_game_relative_path)
                != Some(request.content)
        {
            return Err(refuse(OrdinaryTextureResourceCause::UnauthenticatedContent));
        }
        // This exact path was bound to its admitted winner when registered.
        // Native hits retain the allocation without resolving the file again.
        if let Some(cached) = self.cached(request) {
            return cached.map_err(refuse);
        }
        let resolved = resolver
            .resolve_texture_file(request.native_path)
            .map_err(|_| refuse(OrdinaryTextureResourceCause::ResolveUnavailable))?;
        if resolver.admitted_content_key(&resolved.rel_path) != Some(request.content) {
            return Err(refuse(OrdinaryTextureResourceCause::WrongResolvedWinner));
        }
        // read_file rechecks the selected winner and uses the admission lease.
        // Cache hits retain the previously loaded immutable bytes; they do not
        // reread the filesystem or silently load replacement payloads.
        let bytes = resolver
            .read_file(&resolved)
            .map_err(|_| refuse(OrdinaryTextureResourceCause::ReadUnavailable))?;
        self.register_admitted_bytes(
            request,
            bytes.into(),
            requested_options,
            force_pow2_at_new_load,
        )
        .map_err(refuse)
    }

    fn cached(
        &self,
        request: OrdinaryTextureRequest<'_>,
    ) -> Option<Result<Arc<RegisteredOrdinaryTexture>, OrdinaryTextureResourceCause>> {
        if self
            .namespace
            .as_deref()
            .is_some_and(|namespace| namespace != request.content.effective_vfs_manifest_sha256)
        {
            return Some(Err(OrdinaryTextureResourceCause::ChangedSessionNamespace));
        }
        self.entries.get(request.native_path).map(|entry| {
            if entry.content != *request.content {
                Err(OrdinaryTextureResourceCause::ChangedCachedContent)
            } else {
                entry.result.clone()
            }
        })
    }

    fn register_admitted_bytes(
        &mut self,
        request: OrdinaryTextureRequest<'_>,
        encoded: Arc<[u8]>,
        requested_options: OrdinarySpriteTextureLoadOptions,
        force_pow2_at_new_load: Option<bool>,
    ) -> Result<Arc<RegisteredOrdinaryTexture>, OrdinaryTextureResourceCause> {
        if let Some(cached) = self.cached(request) {
            return cached;
        }
        validate_path(request.native_path)?;
        if request.content.validate().is_err() {
            return Err(OrdinaryTextureResourceCause::UnauthenticatedContent);
        }
        if encoded.len() as u64 != request.content.payload_bytes
            || Sha256::hex(&encoded) != request.content.payload_sha256
        {
            return Err(OrdinaryTextureResourceCause::PayloadIdentityMismatch);
        }
        let force_pow2 =
            force_pow2_at_new_load.ok_or(OrdinaryTextureResourceCause::MissingLoadSetting)?;
        let result = prepare(request.content, encoded, requested_options, force_pow2);
        // Known-but-unsupported resource branches retain a refusal for the
        // entry. A later ordinary request cannot replace earlier load inputs
        // and turn that same native cache hit into an admitted mip-0 image.
        self.namespace = Some(request.content.effective_vfs_manifest_sha256.clone());
        self.entries.insert(
            request.native_path.to_owned(),
            Registration {
                content: request.content.clone(),
                effective_options: requested_options,
                force_pow2_at_load: force_pow2,
                result: result.clone(),
            },
        );
        result
    }
}

fn prepare(
    content: &UsedContentKey,
    encoded: Arc<[u8]>,
    options: OrdinarySpriteTextureLoadOptions,
    force_pow2: bool,
) -> Result<Arc<RegisteredOrdinaryTexture>, OrdinaryTextureResourceCause> {
    let source = OrdinarySingleMipTextureSource::admit(&encoded, Some(&options), Some(force_pow2))
        .map_err(OrdinaryTextureResourceCause::Source)?;
    let dimensions = source.dimensions();
    let decoded = crate::dds::decode_bytes(source.encoded())
        .map_err(|_| OrdinaryTextureResourceCause::DecodeUnavailable)?;
    if [decoded.image.width as u32, decoded.image.height as u32] != dimensions {
        return Err(OrdinaryTextureResourceCause::DecodedDimensionsDisagree);
    }
    Ok(Arc::new(RegisteredOrdinaryTexture {
        content: content.clone(),
        encoded,
        image: Arc::new(decoded.image),
        effective_options: options,
        force_pow2_at_load: force_pow2,
    }))
}

fn validate_path(path: &str) -> Result<(), OrdinaryTextureResourceCause> {
    if path.is_empty()
        || !path.is_ascii()
        || path.bytes().any(|byte| byte < 32)
        || path.starts_with(['@', '/', '\\'])
        || path.contains(':')
        || path.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(OrdinaryTextureResourceCause::UnsupportedRegistrationPath);
    }
    Ok(())
}

fn rejection(
    request: OrdinaryTextureRequest<'_>,
    cause: OrdinaryTextureResourceCause,
) -> OrdinaryTextureResourceRejection {
    OrdinaryTextureResourceRejection {
        role: request.role,
        native_path: request.native_path.into(),
        cause,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_legacy_lazy_load_assignment_overrides_unknown_or_startup_schedule() {
        for (fields, expected, ordinal) in [
            (
                "legacy_lazy_load = no",
                Some(OrdinarySpriteLoadSchedule::Eager),
                Some(2),
            ),
            (
                "legacy_lazy_load = no legacy_lazy_load = yes",
                Some(OrdinarySpriteLoadSchedule::LegacyDeferred),
                Some(3),
            ),
            (
                "legacy_lazy_load = \"yes\" legacy_lazy_load = no",
                Some(OrdinarySpriteLoadSchedule::Eager),
                Some(3),
            ),
            ("", None, None),
        ] {
            let fixture = RequestFixture::new(&format!("textureFile = gfx/a.dds {fields}"));
            let profile =
                OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                    .unwrap();
            assert_eq!(profile.load_schedule, expected);
            assert_eq!(profile.load_schedule_field_ordinal, ordinal);
        }
        for spelling in ["true", "YES", "1", "yes_extra"] {
            let fixture = RequestFixture::new(&format!(
                "legacy_lazy_load = {spelling} legacy_lazy_load = no"
            ));
            let error =
                OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                    .unwrap_err();
            assert_eq!(
                error.cause,
                OrdinarySpriteRequestCause::UnsupportedBooleanValue
            );
            assert_eq!(error.field_ordinal, Some(1));
            assert_eq!(error.role, None);
        }
    }

    #[test]
    fn deferred_static_type_initializes_mask_before_size_getter_and_reuses_load_policy() {
        use crate::container_sprite_resource_schedule::{
            InitializedOrdinarySpriteTypeResources, OrdinarySpriteResourceBinding,
        };
        let fixture = RequestFixture::new(
            "textureFile = gfx/a.dds masking_texture = gfx/a.dds legacy_lazy_load = yes",
        );
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap();
        let records = crate::container_animation_records::AdmittedContainerAnimations::parse(
            &fixture.sprite.raw,
            requests.source.clone(),
            None,
        )
        .unwrap();
        let original = OrdinarySpriteTextureRegistry::new_viewer_session();
        let (initialized, bundle) =
            InitializedOrdinarySpriteTypeResources::stage_type_initialization(
                &original,
                &fixture.resolver,
                &requests,
                &records,
                None,
                Some(false),
                None,
            )
            .unwrap();
        assert!(original.retained_load_inputs("gfx/a.dds").is_none());
        assert!(matches!(
            bundle.base,
            OrdinarySpriteResourceBinding::DeferredBase(_)
        ));
        let OrdinarySpriteResourceBinding::Loaded(mask) = &bundle.masking_texture else {
            panic!("mask must initialize eagerly");
        };
        let (after_getter, next) = bundle
            .stage_size_getter(&initialized, &fixture.resolver, Some(true))
            .unwrap();
        let OrdinarySpriteResourceBinding::Loaded(base) = &next.base else {
            panic!("getter must load base");
        };
        assert!(Arc::ptr_eq(base, mask));
        assert!(!base.force_pow2_at_load());
        assert!(matches!(
            bundle.base,
            OrdinarySpriteResourceBinding::DeferredBase(_)
        ));
        let (_, repeated) = next
            .stage_size_getter(&after_getter, &fixture.resolver, None)
            .unwrap();
        assert_eq!(next, repeated);
    }

    #[test]
    fn eager_schedule_initializes_all_vector_roles_only_after_program_admission() {
        use crate::container_sprite_resource_schedule::{
            InitializedOrdinarySpriteTypeResources, OrdinarySpriteResourceBinding,
            OrdinarySpriteScheduleRejection,
        };
        let fixture = RequestFixture::new(concat!(
            "textureFile = gfx/a.dds masking_texture = gfx/mask.dds legacy_lazy_load = no\n",
            "animation = { animationtype = pulsing animationmaskfile = gfx/mask.dds animationtexturefile = gfx/a.dds }\n",
            "animation = { animationtype = pulsing animationmaskfile = gfx/mask.dds animationtexturefile = gfx/a.dds }\n",
            "animation = { animationtype = pulsing animationmaskfile = gfx/mask.dds animationtexturefile = gfx/a.dds }\n",
        ));
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap();
        let records = crate::container_animation_records::AdmittedContainerAnimations::parse(
            &fixture.sprite.raw,
            requests.source.clone(),
            None,
        )
        .unwrap();
        let original = OrdinarySpriteTextureRegistry::new_viewer_session();
        assert_eq!(
            InitializedOrdinarySpriteTypeResources::stage_type_initialization(
                &original,
                &fixture.resolver,
                &requests,
                &records,
                Some(OrdinarySpriteLoadSchedule::LegacyDeferred),
                Some(false),
                None
            )
            .unwrap_err(),
            OrdinarySpriteScheduleRejection::AnimationProgramInitializationUnavailable
        );
        assert!(original.entries.is_empty());
        let (_, bundle) = InitializedOrdinarySpriteTypeResources::stage_type_initialization(
            &original,
            &fixture.resolver,
            &requests,
            &records,
            Some(OrdinarySpriteLoadSchedule::LegacyDeferred),
            Some(false),
            Some(true),
        )
        .unwrap();
        assert_eq!(bundle.schedule, OrdinarySpriteLoadSchedule::Eager);
        assert_eq!(bundle.animations.len(), 3);
        assert!(bundle.animations.iter().all(|pair| matches!(
            pair.mask,
            OrdinarySpriteResourceBinding::Loaded(_)
        ) && matches!(
            pair.animated,
            OrdinarySpriteResourceBinding::Loaded(_)
        )));
        assert!(original.entries.is_empty());
    }

    #[test]
    fn native_initialization_category_preserves_actual_loader_case_and_last_assignment() {
        for (fields, expected, ordinal) in [
            ("", 0, None),
            ("loadType = FRONTEND", 1, Some(1)),
            ("loadType = BACKEND", 2, Some(1)),
            ("loadType = \"INGAME\"", 3, Some(1)),
            ("loadType = FRONTEND loadType = ingame", 0, Some(2)),
            ("loadType = FRONTEND_extra", 0, Some(1)),
        ] {
            let fixture = RequestFixture::new(fields);
            let requests =
                OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                    .unwrap();
            assert_eq!(requests.initialization_category, expected);
            assert_eq!(requests.initialization_category_field_ordinal, ordinal);
        }
        let malformed = RequestFixture::new("loadType = @phase");
        assert_eq!(
            OrdinarySpriteTextureRequests::from_definition(&malformed.resolver, &malformed.sprite)
                .unwrap_err()
                .cause,
            OrdinarySpriteRequestCause::UnsupportedMacroReference
        );
    }

    #[test]
    fn retained_definition_rejects_different_same_file_rows_without_texture_reread() {
        let source = concat!(
            "spriteTypes = {\n",
            "spriteType = { name = test textureFile = gfx/a.dds }\n",
            "spriteType = { name = other textureFile = gfx/a.dds }\n",
            "}\n",
        )
        .to_owned();
        let fixture = RequestFixture::from_source(source.clone(), "test");
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap();
        let mut registry = crate::gfx::SpriteRegistry::new();
        registry
            .parse_content(source.as_bytes(), "interface/test.gfx")
            .unwrap();
        assert!(!requests.matches_definition(&fixture.resolver, registry.get("other").unwrap()));
        std::fs::remove_file(fixture.game.join("gfx/a.dds")).unwrap();
        assert!(requests.matches_definition(&fixture.resolver, &fixture.sprite));
        let mut drifted = fixture.sprite.clone();
        drifted.no_of_frames += 1;
        assert!(!requests.matches_definition(&fixture.resolver, &drifted));
        let different_admission = RequestFixture::new("textureFile = gfx/a.dds");
        assert!(!requests.matches_definition(&different_admission.resolver, &fixture.sprite));
    }

    #[test]
    fn deferred_type_can_initialize_without_resource_policy_but_getter_cannot_load_without_it() {
        use crate::container_sprite_resource_schedule::{
            InitializedOrdinarySpriteTypeResources, OrdinarySpriteResourceBinding,
            OrdinarySpriteScheduleRejection,
        };
        let fixture = RequestFixture::new("textureFile = gfx/a.dds legacy_lazy_load = yes");
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap();
        let records = crate::container_animation_records::AdmittedContainerAnimations::parse(
            &fixture.sprite.raw,
            requests.source.clone(),
            None,
        )
        .unwrap();
        let original = OrdinarySpriteTextureRegistry::new_viewer_session();
        let (initialized, bundle) =
            InitializedOrdinarySpriteTypeResources::stage_type_initialization(
                &original,
                &fixture.resolver,
                &requests,
                &records,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(initialized, original);
        assert!(matches!(
            bundle.base,
            OrdinarySpriteResourceBinding::DeferredBase(_)
        ));
        let error = bundle
            .stage_size_getter(&initialized, &fixture.resolver, None)
            .unwrap_err();
        assert!(matches!(
            error,
            OrdinarySpriteScheduleRejection::Resource(OrdinaryTextureResourceRejection {
                role: OrdinaryTextureRole::Base,
                cause: OrdinaryTextureResourceCause::MissingLoadSetting,
                ..
            })
        ));
        assert_eq!(initialized, original);
        let (loaded, _) = bundle
            .stage_size_getter(&initialized, &fixture.resolver, Some(false))
            .unwrap();
        assert!(loaded.retained_load_inputs("gfx/a.dds").is_some());
        assert_eq!(initialized, original);
    }

    #[test]
    fn skipped_animation_resources_keep_nonempty_unresolved_names_and_vector_order() {
        use crate::container_sprite_resource_schedule::{
            InitializedOrdinarySpriteTypeResources, OrdinarySpriteResourceBinding,
        };
        let fixture = RequestFixture::new(concat!(
            "legacy_lazy_load = yes ",
            "animation = { animationtype = pulsing animationmaskfile = gfx/not_admitted.dds } ",
            "animation = { animationtype = pulsing animationtexturefile = gfx/also_not_admitted.dds }",
        ));
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap();
        let records = crate::container_animation_records::AdmittedContainerAnimations::parse(
            &fixture.sprite.raw,
            requests.source.clone(),
            None,
        )
        .unwrap();
        let original = OrdinarySpriteTextureRegistry::new_viewer_session();
        let (next, bundle) = InitializedOrdinarySpriteTypeResources::stage_type_initialization(
            &original,
            &fixture.resolver,
            &requests,
            &records,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(next, original);
        assert_eq!(bundle.animations.len(), 2);
        assert_eq!(bundle.animations[0].authored_ordinal, 0);
        assert_eq!(bundle.animations[1].authored_ordinal, 1);
        assert!(matches!(&bundle.animations[0].mask,
            OrdinarySpriteResourceBinding::NotInitializedByDeferredSchedule { native_path } if native_path == "gfx/not_admitted.dds"));
        assert_eq!(
            bundle.animations[0].animated,
            OrdinarySpriteResourceBinding::Absent
        );
        assert!(matches!(&bundle.animations[1].animated,
            OrdinarySpriteResourceBinding::NotInitializedByDeferredSchedule { native_path } if native_path == "gfx/also_not_admitted.dds"));
        assert_eq!(
            bundle.animations[1].mask,
            OrdinarySpriteResourceBinding::Absent
        );
    }

    #[test]
    fn schedule_keeps_empty_and_absent_and_requires_the_constructor_input_when_unassigned() {
        use crate::container_sprite_resource_schedule::{
            InitializedOrdinarySpriteTypeResources, OrdinarySpriteResourceBinding,
            OrdinarySpriteScheduleRejection,
        };
        let fixture = RequestFixture::new("textureFile = \"\"");
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap();
        let records = crate::container_animation_records::AdmittedContainerAnimations::parse(
            &fixture.sprite.raw,
            requests.source.clone(),
            None,
        )
        .unwrap();
        let original = OrdinarySpriteTextureRegistry::new_viewer_session();
        assert_eq!(
            InitializedOrdinarySpriteTypeResources::stage_type_initialization(
                &original,
                &fixture.resolver,
                &requests,
                &records,
                None,
                None,
                None,
            )
            .unwrap_err(),
            OrdinarySpriteScheduleRejection::ConstructorLoadScheduleUnavailable
        );
        let (next, bundle) = InitializedOrdinarySpriteTypeResources::stage_type_initialization(
            &original,
            &fixture.resolver,
            &requests,
            &records,
            Some(OrdinarySpriteLoadSchedule::LegacyDeferred),
            None,
            None,
        )
        .unwrap();
        assert_eq!(next, original);
        assert_eq!(
            bundle.base,
            OrdinarySpriteResourceBinding::AuthoredEmpty { field_ordinal: 1 }
        );
        assert_eq!(
            bundle.masking_texture,
            OrdinarySpriteResourceBinding::Absent
        );
        assert_eq!(bundle.schedule, OrdinarySpriteLoadSchedule::LegacyDeferred);
    }

    #[test]
    fn first_resource_failure_tracks_native_schedule_and_never_publishes_partial_registry() {
        use crate::container_sprite_resource_schedule::{
            InitializedOrdinarySpriteTypeResources, OrdinarySpriteScheduleRejection,
        };
        for (legacy, expected) in [
            ("no", OrdinaryTextureRole::Base),
            ("yes", OrdinaryTextureRole::MaskingTexture),
        ] {
            let fixture = RequestFixture::new(&format!(
                "textureFile = gfx/a.dds masking_texture = gfx/mask.dds legacy_lazy_load = {legacy}"
            ));
            let requests =
                OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                    .unwrap();
            let records = crate::container_animation_records::AdmittedContainerAnimations::parse(
                &fixture.sprite.raw,
                requests.source.clone(),
                None,
            )
            .unwrap();
            let original = OrdinarySpriteTextureRegistry::new_viewer_session();
            let failure = InitializedOrdinarySpriteTypeResources::stage_type_initialization(
                &original,
                &fixture.resolver,
                &requests,
                &records,
                None,
                Some(true),
                None,
            )
            .unwrap_err();
            assert!(matches!(failure, OrdinarySpriteScheduleRejection::Resource(
                OrdinaryTextureResourceRejection { role, cause: OrdinaryTextureResourceCause::Source(OrdinaryTextureSourceRejection::UnsupportedPadding), .. }
            ) if role == expected));
            assert_eq!(
                original,
                OrdinarySpriteTextureRegistry::new_viewer_session()
            );
        }
    }

    fn owner_initialization_inputs(
        fixture: &RequestFixture,
        operation_sequence: u64,
    ) -> (
        OrdinarySpriteTextureRequests,
        crate::container_animation_records::AdmittedContainerAnimations,
        OrdinarySpriteTypeInitializationOperation,
    ) {
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite).unwrap();
        let records = crate::container_animation_records::AdmittedContainerAnimations::parse(
            &fixture.sprite.raw,
            requests.source.clone(),
            None,
        )
        .unwrap();
        // Explicit operation primitive only: this test-only capability is not
        // evidence that the production library builder has reached this row.
        let operation = OrdinarySpriteTypeInitializationOperation::fixture(
            &fixture.resolver,
            &requests,
            &records,
            operation_sequence,
        );
        (requests, records, operation)
    }

    #[test]
    fn retained_type_lookup_requires_reached_library_initialization() {
        let fixture = RequestFixture::new("textureFile = gfx/a.dds legacy_lazy_load = no");
        let mut owner = ViewerSpriteResources::new_viewer_session();
        let original = owner.clone();
        assert_eq!(
            owner
                .type_resources_for(&fixture.resolver, &fixture.sprite)
                .unwrap_err(),
            "InitializationAuthorityUnavailable"
        );
        assert_eq!(
            owner
                .stage_icon_size_getter(&fixture.resolver, &fixture.sprite, Some(false))
                .unwrap_err(),
            "InitializationAuthorityUnavailable"
        );
        assert_eq!(owner, original);
        assert!(owner.ordinary.entries.is_empty());
    }

    #[test]
    fn reached_type_initialization_keeps_once_only_bundle_and_first_operation() {
        use crate::container_sprite_resource_schedule::OrdinarySpriteResourceBinding;
        let fixture = RequestFixture::new("textureFile = gfx/a.dds legacy_lazy_load = no");
        let (requests, records, operation) = owner_initialization_inputs(&fixture, 7);
        let original = ViewerSpriteResources::new_viewer_session();
        let initialized = original
            .stage_reached_type_initialization(
                &fixture.resolver,
                &operation,
                &requests,
                &records,
                None,
                Some(false),
                None,
            )
            .unwrap();
        assert!(original.ordinary_types.is_empty());
        let bundle = initialized
            .type_resources_for(&fixture.resolver, &fixture.sprite)
            .unwrap();
        assert_eq!(bundle.requests.source, requests.source);
        assert_eq!(
            bundle.requests.sprite_source_ordinal,
            requests.sprite_source_ordinal
        );
        let OrdinarySpriteResourceBinding::Loaded(first) = &bundle.base else {
            panic!("loaded base")
        };
        let later_operation = OrdinarySpriteTypeInitializationOperation::fixture(
            &fixture.resolver,
            &requests,
            &records,
            19,
        );
        let repeated = initialized
            .stage_reached_type_initialization(
                &fixture.resolver,
                &later_operation,
                &requests,
                &records,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(repeated, initialized);
        let row = &repeated.ordinary_types
            [&OrdinarySpriteTypeKey::from_requests(&operation.manifest_namespace, &requests)];
        assert_eq!(row.first_operation.operation_sequence, 7);
        let OrdinarySpriteResourceBinding::Loaded(second) = &row.resources.base else {
            panic!("retained base")
        };
        assert!(Arc::ptr_eq(first, second));
    }

    #[test]
    fn icon_size_getter_stages_existing_deferred_type_atomically() {
        use crate::container_sprite_resource_schedule::OrdinarySpriteResourceBinding;
        let fixture = RequestFixture::new("textureFile = gfx/a.dds legacy_lazy_load = yes");
        let (requests, records, operation) = owner_initialization_inputs(&fixture, 0);
        let mut owner = ViewerSpriteResources::new_viewer_session()
            .stage_reached_type_initialization(
                &fixture.resolver,
                &operation,
                &requests,
                &records,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(owner.ordinary.entries.is_empty());
        let before_getter = owner.clone();
        assert!(owner
            .stage_icon_size_getter(&fixture.resolver, &fixture.sprite, None)
            .unwrap_err()
            .contains("MissingLoadSetting"));
        assert_eq!(owner, before_getter);
        let loaded = owner
            .stage_icon_size_getter(&fixture.resolver, &fixture.sprite, Some(false))
            .unwrap();
        assert!(matches!(
            before_getter
                .type_resources_for(&fixture.resolver, &fixture.sprite)
                .unwrap()
                .base,
            OrdinarySpriteResourceBinding::DeferredBase(_)
        ));
        let OrdinarySpriteResourceBinding::Loaded(first) = &loaded.base else {
            panic!("loaded base")
        };
        let after_getter = owner.clone();
        let repeated = owner
            .stage_icon_size_getter(&fixture.resolver, &fixture.sprite, Some(true))
            .unwrap();
        assert_eq!(owner, after_getter);
        let OrdinarySpriteResourceBinding::Loaded(second) = &repeated.base else {
            panic!("retained base")
        };
        assert!(Arc::ptr_eq(first, second));
        assert!(!second.force_pow2_at_load());
        let repeated_init = owner
            .stage_reached_type_initialization(
                &fixture.resolver,
                &operation,
                &requests,
                &records,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(repeated_init, owner);
    }

    #[test]
    fn reached_type_failure_discards_the_registry_prefix_and_type() {
        let fixture = RequestFixture::new(concat!(
            "textureFile = gfx/a.dds legacy_lazy_load = no\n",
            "animation = { animationtype = pulsing animationmaskfile = gfx/unadmitted.dds }\n",
        ));
        let (requests, records, operation) = owner_initialization_inputs(&fixture, 0);
        let owner = ViewerSpriteResources::new_viewer_session();
        let before = owner.clone();
        let failure = owner
            .stage_reached_type_initialization(
                &fixture.resolver,
                &operation,
                &requests,
                &records,
                None,
                Some(false),
                Some(true),
            )
            .unwrap_err();
        assert!(matches!(failure, OrdinarySpriteTypeOwnerRejection::Schedule(
                crate::container_sprite_resource_schedule::OrdinarySpriteScheduleRejection::AnimationWinnerUnavailable { .. }
            )));
        assert_eq!(owner, before);
        assert!(owner.ordinary.entries.is_empty());
        assert!(owner.ordinary_types.is_empty());
    }

    #[test]
    fn type_operation_joins_and_conflicting_inputs_cannot_replace_initialized_type() {
        let fixture = RequestFixture::new("textureFile = gfx/a.dds legacy_lazy_load = no");
        let (requests, records, operation) = owner_initialization_inputs(&fixture, 0);
        let owner = ViewerSpriteResources::new_viewer_session()
            .stage_reached_type_initialization(
                &fixture.resolver,
                &operation,
                &requests,
                &records,
                None,
                Some(false),
                None,
            )
            .unwrap();
        let before = owner.clone();
        for changed in 0..4 {
            let mut wrong = operation.clone();
            match changed {
                0 => wrong.manifest_namespace = "f".repeat(64),
                1 => wrong.source.payload_sha256 = "e".repeat(64),
                2 => wrong.type_ordinal[1] += 1,
                _ => wrong.category += 1,
            }
            assert_eq!(
                owner
                    .stage_reached_type_initialization(
                        &fixture.resolver,
                        &wrong,
                        &requests,
                        &records,
                        None,
                        None,
                        None,
                    )
                    .unwrap_err(),
                OrdinarySpriteTypeOwnerRejection::OperationIdentityMismatch
            );
        }
        let mut conflicting_requests = requests.clone();
        conflicting_requests.base = OrdinarySpriteTextureField::Absent;
        assert_eq!(
            owner
                .stage_reached_type_initialization(
                    &fixture.resolver,
                    &operation,
                    &conflicting_requests,
                    &records,
                    None,
                    None,
                    None,
                )
                .unwrap_err(),
            OrdinarySpriteTypeOwnerRejection::ConflictingTypeIdentity
        );
        assert_eq!(
            owner
                .stage_reached_type_initialization(
                    &fixture.resolver,
                    &operation,
                    &requests,
                    &records,
                    Some(OrdinarySpriteLoadSchedule::LegacyDeferred),
                    None,
                    None,
                )
                .unwrap_err(),
            OrdinarySpriteTypeOwnerRejection::ConflictingTypeIdentity
        );
        assert_eq!(owner, before);
    }

    #[test]
    fn same_source_other_row_and_different_admission_do_not_borrow_initialized_type() {
        let fixture = RequestFixture::new(concat!(
            "textureFile = gfx/a.dds legacy_lazy_load = no\n}\n",
            "spriteType = { name = other textureFile = gfx/a.dds legacy_lazy_load = no",
        ));
        let (requests, records, operation) = owner_initialization_inputs(&fixture, 0);
        let owner = ViewerSpriteResources::new_viewer_session()
            .stage_reached_type_initialization(
                &fixture.resolver,
                &operation,
                &requests,
                &records,
                None,
                Some(false),
                None,
            )
            .unwrap();
        let file = fixture.resolver.resolve_file("interface/test.gfx").unwrap();
        let bytes = fixture.resolver.read_file(&file).unwrap();
        let mut registry = crate::gfx::SpriteRegistry::new();
        registry
            .parse_content(&bytes, "interface/test.gfx")
            .unwrap();
        let other = registry.get("other").unwrap();
        assert_eq!(
            owner
                .type_resources_for(&fixture.resolver, other)
                .unwrap_err(),
            "InitializationAuthorityUnavailable"
        );
        let different = RequestFixture::new("textureFile = gfx/mask.dds legacy_lazy_load = no");
        assert_eq!(
            owner
                .type_resources_for(&different.resolver, &different.sprite)
                .unwrap_err(),
            "ChangedAdmission"
        );
        let other_requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, other).unwrap();
        assert_ne!(
            requests.sprite_source_ordinal,
            other_requests.sprite_source_ordinal
        );
        let other_records = crate::container_animation_records::AdmittedContainerAnimations::parse(
            &other.raw,
            other_requests.source.clone(),
            None,
        )
        .unwrap();
        let other_operation = OrdinarySpriteTypeInitializationOperation::fixture(
            &fixture.resolver,
            &other_requests,
            &other_records,
            1,
        );
        let both = owner
            .stage_reached_type_initialization(
                &fixture.resolver,
                &other_operation,
                &other_requests,
                &other_records,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(both.ordinary_types.len(), 2);
        assert!(both
            .type_resources_for(&fixture.resolver, &fixture.sprite)
            .is_ok());
        assert!(both.type_resources_for(&fixture.resolver, other).is_ok());
    }

    #[test]
    fn initialized_type_namespace_survives_a_witnessed_no_resource_initialization() {
        let fixture = RequestFixture::new("textureFile = gfx/a.dds legacy_lazy_load = yes");
        let (requests, records, operation) = owner_initialization_inputs(&fixture, 0);
        let owner = ViewerSpriteResources::new_viewer_session()
            .stage_reached_type_initialization(
                &fixture.resolver,
                &operation,
                &requests,
                &records,
                None,
                None,
                None,
            )
            .unwrap();
        assert!(owner.ordinary.namespace.is_none());
        assert_eq!(
            owner.type_namespace.as_deref(),
            Some(ViewerSpriteResources::admitted_type_manifest(&fixture.resolver).unwrap())
        );
        let different = RequestFixture::new("textureFile = gfx/mask.dds legacy_lazy_load = yes");
        let (other_requests, other_records, other_operation) =
            owner_initialization_inputs(&different, 0);
        assert_eq!(
            owner
                .stage_reached_type_initialization(
                    &different.resolver,
                    &other_operation,
                    &other_requests,
                    &other_records,
                    None,
                    None,
                    None,
                )
                .unwrap_err(),
            OrdinarySpriteTypeOwnerRejection::ChangedAdmission
        );
    }

    struct RequestFixture {
        game: std::path::PathBuf,
        resolver: AssetResolver,
        sprite: SpriteDefinition,
    }

    impl RequestFixture {
        fn new(fields: &str) -> Self {
            let source = format!("ignored = yes\nspriteTypes = {{\n spriteType = {{ name = test\n{fields}\n }}\n}}\n");
            Self::from_source(source, "test")
        }

        fn from_source(source: String, selected: &str) -> Self {
            let game = std::env::temp_dir().join(format!(
                "ordinary-sprite-request-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(game.join("gfx")).unwrap();
            std::fs::create_dir_all(game.join("interface")).unwrap();
            std::fs::write(game.join("interface/test.gfx"), source.as_bytes()).unwrap();
            std::fs::write(game.join("gfx/a.dds"), dds()).unwrap();
            std::fs::write(game.join("gfx/mask.dds"), dds()).unwrap();
            let (files, admission) = crate::test_admission::admit_vanilla(
                &game,
                "l_english",
                &["interface/test.gfx", "gfx/a.dds", "gfx/mask.dds"],
                "ordinary-sprite-request",
            );
            let resolver = AssetResolver::from_content_admission(&files, &admission).unwrap();
            let mut registry = crate::gfx::SpriteRegistry::new();
            registry
                .parse_content(source.as_bytes(), "interface/test.gfx")
                .unwrap();
            let sprite = registry.get(selected).unwrap().clone();
            Self {
                game,
                resolver,
                sprite,
            }
        }
    }

    impl Drop for RequestFixture {
        fn drop(&mut self) {
            // This fixture owns the unique pid/time directory it created.
            std::fs::remove_dir_all(&self.game).unwrap();
        }
    }

    #[test]
    fn source_requests_keep_last_assignments_ordinals_and_independent_winners() {
        let fixture = RequestFixture::new(concat!(
            "textureFile = \"gfx/missing.dds\"\n",
            "masking_texture = \"gfx/missing_mask.dds\"\n",
            "textureFile = \"gfx/a.dds\"\n",
            "masking_texture = \"gfx/mask.dds\"\n",
        ));
        // The generic family helper demonstrably selects the wrong operand
        // for this native ordinary branch; extraction must not use it.
        assert_eq!(fixture.sprite.primary_texture(), Some("gfx/missing.dds"));
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap();
        assert_eq!(requests.sprite_source_ordinal, [1, 0]);
        assert_eq!(
            requests.source.normalized_game_relative_path,
            "interface/test.gfx"
        );
        let OrdinarySpriteTextureField::File(base) = &requests.base else {
            panic!("base file");
        };
        let OrdinarySpriteTextureField::File(mask) = &requests.masking_texture else {
            panic!("mask file");
        };
        assert_eq!(
            (base.native_path.as_str(), base.field_ordinal),
            ("gfx/a.dds", 3)
        );
        assert_eq!(
            (mask.native_path.as_str(), mask.field_ordinal),
            ("gfx/mask.dds", 4)
        );
        assert_eq!(base.content.normalized_game_relative_path, "gfx/a.dds");
        assert_eq!(mask.content.normalized_game_relative_path, "gfx/mask.dds");
        let mut registry = OrdinarySpriteTextureRegistry::new_viewer_session();
        let image = registry
            .get_or_register_ordinary(&fixture.resolver, base.borrowed(), Some(false))
            .unwrap();
        assert_eq!(image.content(), &base.content);
        let wrong_mask = OrdinaryTextureRequest {
            role: mask.role,
            native_path: &mask.native_path,
            content: &base.content,
        };
        assert_eq!(
            registry
                .get_or_register_ordinary(&fixture.resolver, wrong_mask, Some(false))
                .unwrap_err()
                .cause,
            OrdinaryTextureResourceCause::WrongResolvedWinner
        );
        assert!(registry
            .get_or_register_ordinary(&fixture.resolver, mask.borrowed(), Some(false))
            .is_ok());
    }

    #[test]
    fn extracted_requests_keep_cached_native_path_options_and_initial_policy() {
        let fixture = RequestFixture::new("textureFile = gfx/a.dds\nmasking_texture = gfx/a.dds");
        let requests =
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap();
        let OrdinarySpriteTextureField::File(base) = &requests.base else {
            panic!("base");
        };
        let OrdinarySpriteTextureField::File(mask) = &requests.masking_texture else {
            panic!("mask");
        };
        let mut registry = OrdinarySpriteTextureRegistry::new_viewer_session();
        let first = registry
            .get_or_register_ordinary(&fixture.resolver, base.borrowed(), Some(false))
            .unwrap();
        let hit = registry
            .get_or_register_ordinary(&fixture.resolver, mask.borrowed(), Some(true))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &hit));
        assert!(!hit.force_pow2_at_load());
        let mut refused = OrdinarySpriteTextureRegistry::new_viewer_session();
        let mut initial = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        initial.dds_mips_to_skip = 1;
        let first_error = refused
            .get_or_register_file_request(&fixture.resolver, mask.borrowed(), initial, Some(false))
            .unwrap_err();
        let hit_error = refused
            .get_or_register_ordinary(&fixture.resolver, base.borrowed(), Some(true))
            .unwrap_err();
        assert_eq!(first_error.cause, hit_error.cause);
        assert_eq!(
            refused.retained_load_inputs(&base.native_path),
            Some((initial, false))
        );
    }

    #[test]
    fn source_request_absent_empty_and_unknown_tokens_stay_distinct() {
        let empty = RequestFixture::new("textureFile = gfx/a.dds\ntextureFile = \"\"");
        let result =
            OrdinarySpriteTextureRequests::from_definition(&empty.resolver, &empty.sprite).unwrap();
        assert_eq!(
            result.base,
            OrdinarySpriteTextureField::Empty { field_ordinal: 2 }
        );
        assert_eq!(result.masking_texture, OrdinarySpriteTextureField::Absent);
        for (field, cause) in [
            (
                "textureFile = { gfx/a.dds }",
                OrdinarySpriteRequestCause::UnsupportedFieldType,
            ),
            (
                "texturefile = gfx/a.dds",
                OrdinarySpriteRequestCause::UnsupportedFieldCaseOrQuotedKey,
            ),
            (
                "\"textureFile\" = gfx/a.dds",
                OrdinarySpriteRequestCause::UnsupportedFieldCaseOrQuotedKey,
            ),
            (
                "textureFile = @filename",
                OrdinarySpriteRequestCause::UnsupportedMacroReference,
            ),
            (
                "textureFile = \"gfx/\\a.dds\"",
                OrdinarySpriteRequestCause::UnsupportedSourceTokenSpelling,
            ),
            (
                "textureFile = \"gfx/a\\ndds\"",
                OrdinarySpriteRequestCause::UnsupportedSourceTokenSpelling,
            ),
            (
                "textureFile = \"gfx/a\ndds\"",
                OrdinarySpriteRequestCause::NativeUnreadableString,
            ),
        ] {
            // An invalid assignment is not skipped even if a later assignment
            // could supply a usable path: native error effects are not erased.
            let fixture = RequestFixture::new(&format!("{field}\ntextureFile = gfx/a.dds"));
            let error =
                OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                    .unwrap_err();
            assert_eq!(error.cause, cause);
            assert_eq!(error.sprite_source_ordinal, Some([1, 0]));
            assert_eq!(error.field_ordinal, Some(1));
            assert_eq!(error.role, Some(OrdinaryTextureRole::Base));
        }
    }

    #[test]
    fn source_request_refuses_definition_drift_and_missing_selected_texture() {
        let mut fixture = RequestFixture::new("textureFile = gfx/a.dds");
        fixture.sprite.raw =
            crate::paradox_text::parse_str("name = test textureFile = gfx/mask.dds").root;
        assert_eq!(
            OrdinarySpriteTextureRequests::from_definition(&fixture.resolver, &fixture.sprite)
                .unwrap_err()
                .cause,
            OrdinarySpriteRequestCause::SourceDefinitionMismatch
        );
        let missing =
            RequestFixture::new("textureFile = gfx/a.dds\nmasking_texture = gfx/missing.dds");
        let error =
            OrdinarySpriteTextureRequests::from_definition(&missing.resolver, &missing.sprite)
                .unwrap_err();
        assert_eq!(error.role, Some(OrdinaryTextureRole::MaskingTexture));
        assert_eq!(error.field_ordinal, Some(2));
        assert_eq!(
            error.cause,
            OrdinarySpriteRequestCause::Resource(OrdinaryTextureResourceCause::ResolveUnavailable)
        );
    }

    fn dds() -> Arc<[u8]> {
        let mut bytes = vec![0; 128 + 3 * 2 * 4];
        bytes[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [
            (4, 124_u32),
            (8, 0x100f),
            (12, 2),
            (16, 3),
            (28, 1),
            (76, 32),
            (80, 0x41),
            (88, 32),
            (92, 0x00ff_0000),
            (96, 0x0000_ff00),
            (100, 0x0000_00ff),
            (104, 0xff00_0000),
            (108, 0x1000),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[128..132].copy_from_slice(&[3, 5, 7, 11]);
        bytes.into()
    }

    fn content(path: &str, bytes: &[u8]) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            stored_game_relative_path: path.into(),
            normalized_game_relative_path: path.into(),
            winning_layer_ordinal: 0,
            layer_kind: replay_core::ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: Sha256::hex(bytes),
            payload_sha256: Sha256::hex(bytes),
            payload_bytes: bytes.len() as u64,
        }
    }

    fn request<'a>(
        path: &'a str,
        content: &'a UsedContentKey,
        role: OrdinaryTextureRole,
    ) -> OrdinaryTextureRequest<'a> {
        OrdinaryTextureRequest {
            role,
            native_path: path,
            content,
        }
    }

    #[test]
    fn cache_hit_preserves_load_policy_bytes_and_resource_across_roles() {
        let bytes = dds();
        let key = content("gfx/a.dds", &bytes);
        let mut registry = OrdinarySpriteTextureRegistry::new_viewer_session();
        let options = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        let first = registry
            .register_admitted_bytes(
                request("gfx/a.dds", &key, OrdinaryTextureRole::Base),
                Arc::clone(&bytes),
                options,
                Some(false),
            )
            .unwrap();
        assert_eq!(&first.image().pixels[..4], &[7, 5, 3, 11]);
        assert_eq!(first.dimensions(), [3, 2]);
        assert_eq!(first.content(), &key);
        assert_eq!(first.encoded(), bytes.as_ref());
        for role in [
            OrdinaryTextureRole::MaskingTexture,
            OrdinaryTextureRole::AnimationMask(0),
            OrdinaryTextureRole::AnimationTexture(0),
            OrdinaryTextureRole::AnimationMask(1),
            OrdinaryTextureRole::AnimationTexture(1),
        ] {
            let later_options = OrdinarySpriteTextureLoadOptions {
                srgb: true,
                ..options
            };
            let cached = registry
                .register_admitted_bytes(
                    request("gfx/a.dds", &key, role),
                    Arc::clone(&bytes),
                    later_options,
                    Some(true),
                )
                .unwrap();
            assert!(Arc::ptr_eq(&first, &cached));
            assert_eq!(cached.effective_options(), options);
            assert!(!cached.force_pow2_at_load());
        }
        assert_eq!(registry.entries.len(), 1);
    }

    #[test]
    fn native_path_bytes_partition_entries_but_winner_identity_does_not() {
        let bytes = dds();
        let key = content("gfx/a.dds", &bytes);
        let mut registry = OrdinarySpriteTextureRegistry::new_viewer_session();
        let options = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        let first = registry
            .register_admitted_bytes(
                request("gfx/a.dds", &key, OrdinaryTextureRole::Base),
                Arc::clone(&bytes),
                options,
                Some(false),
            )
            .unwrap();
        let alias = registry
            .register_admitted_bytes(
                request("GFX/A.dds", &key, OrdinaryTextureRole::Base),
                Arc::clone(&bytes),
                options,
                Some(false),
            )
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &alias));
        assert_eq!(registry.entries.len(), 2);
        let mut changed = key.clone();
        changed.source_sha256 = "b".repeat(64);
        assert_eq!(
            registry
                .register_admitted_bytes(
                    request("gfx/a.dds", &changed, OrdinaryTextureRole::Base),
                    Arc::clone(&bytes),
                    options,
                    Some(false),
                )
                .unwrap_err(),
            OrdinaryTextureResourceCause::ChangedCachedContent
        );
        changed.effective_vfs_manifest_sha256 = "c".repeat(64);
        assert_eq!(
            registry
                .register_admitted_bytes(
                    request("gfx/new.dds", &changed, OrdinaryTextureRole::Base),
                    bytes,
                    options,
                    Some(false),
                )
                .unwrap_err(),
            OrdinaryTextureResourceCause::ChangedSessionNamespace
        );
    }

    #[test]
    fn unsupported_first_registration_never_becomes_ordinary_mip_zero_on_a_hit() {
        let bytes = dds();
        let key = content("gfx/a.dds", &bytes);
        let mut registry = OrdinarySpriteTextureRegistry::new_viewer_session();
        let ordinary = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        let first_options = OrdinarySpriteTextureLoadOptions {
            dds_mips_to_skip: 1,
            ..ordinary
        };
        let expected = OrdinaryTextureResourceCause::Source(
            OrdinaryTextureSourceRejection::UnsupportedMipSkip,
        );
        assert_eq!(
            registry
                .register_admitted_bytes(
                    request("gfx/a.dds", &key, OrdinaryTextureRole::AnimationTexture(1)),
                    Arc::clone(&bytes),
                    first_options,
                    Some(false),
                )
                .unwrap_err(),
            expected
        );
        assert_eq!(
            registry
                .register_admitted_bytes(
                    request("gfx/a.dds", &key, OrdinaryTextureRole::Base),
                    bytes,
                    ordinary,
                    Some(false),
                )
                .unwrap_err(),
            expected
        );
        assert_eq!(
            registry.retained_load_inputs("gfx/a.dds"),
            Some((first_options, false))
        );
    }

    #[test]
    fn unknown_or_unauthenticated_inputs_do_not_establish_a_registration() {
        let bytes = dds();
        let key = content("gfx/a.dds", &bytes);
        let mut registry = OrdinarySpriteTextureRegistry::new_viewer_session();
        let options = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        let req = request("gfx/a.dds", &key, OrdinaryTextureRole::AnimationMask(1));
        assert_eq!(
            registry
                .register_admitted_bytes(req, Arc::clone(&bytes), options, None)
                .unwrap_err(),
            OrdinaryTextureResourceCause::MissingLoadSetting
        );
        assert_eq!(
            registry
                .register_admitted_bytes(req, Arc::from(&b"changed"[..]), options, Some(false))
                .unwrap_err(),
            OrdinaryTextureResourceCause::PayloadIdentityMismatch
        );
        let error = registry
            .get_or_register_ordinary(&AssetResolver::new(Vec::new()), req, Some(false))
            .unwrap_err();
        assert_eq!(error.role, OrdinaryTextureRole::AnimationMask(1));
        assert_eq!(
            error.cause,
            OrdinaryTextureResourceCause::UnauthenticatedResolver
        );
        assert!(registry.entries.is_empty());
        assert!(registry.namespace.is_none());
    }

    #[test]
    fn discarded_candidate_does_not_change_presented_registrations() {
        let bytes = dds();
        let key = content("gfx/a.dds", &bytes);
        let options = OrdinarySpriteTextureLoadOptions::from_first_ordinary_registration();
        let mut presented = OrdinarySpriteTextureRegistry::new_viewer_session();
        presented
            .register_admitted_bytes(
                request("gfx/a.dds", &key, OrdinaryTextureRole::Base),
                Arc::clone(&bytes),
                options,
                Some(false),
            )
            .unwrap();
        let mut candidate = presented.clone();
        assert!(candidate
            .register_admitted_bytes(
                request("gfx/other.dds", &key, OrdinaryTextureRole::MaskingTexture),
                bytes,
                options,
                Some(true),
            )
            .is_err());
        assert_eq!(candidate.entries.len(), 2);
        assert_eq!(presented.entries.len(), 1);
        let fresh = OrdinarySpriteTextureRegistry::new_viewer_session();
        assert!(fresh.retained_load_inputs("gfx/a.dds").is_none());
    }

    #[test]
    fn production_registration_reads_admitted_bytes_and_rejects_wrong_role_winner() {
        let game = std::env::temp_dir().join(format!(
            "ordinary-sprite-resource-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(game.join("gfx")).unwrap();
        std::fs::write(game.join("gfx/a.dds"), dds()).unwrap();
        std::fs::write(game.join("gfx/mask.dds"), dds()).unwrap();
        let (files, admission) = crate::test_admission::admit_vanilla(
            &game,
            "l_english",
            &["gfx/a.dds", "gfx/mask.dds"],
            "ordinary-sprite-resource",
        );
        let resolver = AssetResolver::from_content_admission(&files, &admission).unwrap();
        let base = admission.require_used_content("gfx/a.dds").unwrap();
        let mask = admission.require_used_content("gfx/mask.dds").unwrap();
        let mut registry = OrdinarySpriteTextureRegistry::new_viewer_session();
        let first = registry
            .get_or_register_ordinary(
                &resolver,
                request("gfx/a.dds", base, OrdinaryTextureRole::Base),
                Some(false),
            )
            .unwrap();
        assert_eq!(first.encoded(), dds().as_ref());
        assert_eq!(&first.image().pixels[..4], &[7, 5, 3, 11]);
        let error = registry
            .get_or_register_ordinary(
                &resolver,
                request("gfx/mask.dds", base, OrdinaryTextureRole::AnimationMask(0)),
                Some(false),
            )
            .unwrap_err();
        assert_eq!(
            error.cause,
            OrdinaryTextureResourceCause::WrongResolvedWinner
        );
        let second = registry
            .get_or_register_ordinary(
                &resolver,
                request("gfx/mask.dds", mask, OrdinaryTextureRole::AnimationMask(0)),
                Some(false),
            )
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.content(), mask);
        std::fs::remove_file(game.join("gfx/a.dds")).unwrap();
        let retained = registry
            .get_or_register_ordinary(
                &resolver,
                request("gfx/a.dds", base, OrdinaryTextureRole::AnimationTexture(0)),
                None,
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first, &retained));
        std::fs::remove_dir_all(game).unwrap();
    }
}
