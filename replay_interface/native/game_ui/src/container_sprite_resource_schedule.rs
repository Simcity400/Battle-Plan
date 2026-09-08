//! Ordinary CSpriteType resource operations. The caller owns the viewer type
//! table and the outer presentation transaction. Type initialization and the
//! lazy size getter are separate native operations, not raster-pass defaults.
//! Candidate native initialization/order provenance and gaps:
//! extractor/native/runs/arrow_native_ui_20260908/CANDIDATE_RESOURCE_LIFECYCLES.json.
//! Production initialization authority remains unavailable.

use crate::assets::AssetResolver;
use crate::container_animation_records::AdmittedContainerAnimations;
use crate::container_sprite_resources::{
    OrdinarySpriteLoadSchedule, OrdinarySpriteTextureField, OrdinarySpriteTextureRegistry,
    OrdinarySpriteTextureRequests, OrdinaryTextureRequest, OrdinaryTextureResourceRejection,
    OrdinaryTextureRole, OwnedOrdinaryTextureRequest, RegisteredOrdinaryTexture,
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OrdinarySpriteResourceBinding {
    Absent,
    AuthoredEmpty {
        field_ordinal: usize,
    },
    DeferredBase(OwnedOrdinaryTextureRequest),
    /// A nonempty animation filename exists but +1d3 skips the native vector
    /// resource initialization. This is not a missing-file or empty-string fact.
    NotInitializedByDeferredSchedule {
        native_path: String,
    },
    Loaded(Arc<RegisteredOrdinaryTexture>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrdinaryAnimationResourcePair {
    pub authored_ordinal: usize,
    pub mask: OrdinarySpriteResourceBinding,
    pub animated: OrdinarySpriteResourceBinding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InitializedOrdinarySpriteTypeResources {
    pub requests: OrdinarySpriteTextureRequests,
    pub schedule: OrdinarySpriteLoadSchedule,
    pub base: OrdinarySpriteResourceBinding,
    pub masking_texture: OrdinarySpriteResourceBinding,
    /// Every retained native vector row, not only the two shader draw slots.
    pub animations: Vec<OrdinaryAnimationResourcePair>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OrdinarySpriteScheduleRejection {
    ConstructorLoadScheduleUnavailable,
    AnimationSourceMismatch,
    /// At eager init 14222b5f0 builds two animation program variants and may
    /// clear the native record vector before texture requests. The shader
    /// owner must admit the retained-vector branch; it cannot default success.
    AnimationProgramInitializationUnavailable,
    AnimationVectorClearedRequiresLifetimeRebuild,
    AnimationWinnerUnavailable {
        role: OrdinaryTextureRole,
        native_path: String,
    },
    Resource(OrdinaryTextureResourceRejection),
}

impl InitializedOrdinarySpriteTypeResources {
    /// Mirror a reached first 142341040 -> CSpriteType+48 initialization.
    /// The owner must deduplicate by source/type identity before this call.
    /// `constructor_schedule` is the value copied at type construction, before
    /// authored token 655 assignments; None remains unknown if no assignment.
    ///
    /// For nonempty eager animation vectors, Some(true) means the shader owner
    /// has admitted both animation program variants and the native retained-
    /// vector branch. Some(false) names the native clear branch, requiring an
    /// actual animation-count/lifetime reconstruction outside this leaf. No
    /// animation-program premise is needed for the zero-record static branch.
    pub(crate) fn stage_type_initialization(
        presented_registry: &OrdinarySpriteTextureRegistry,
        resolver: &AssetResolver,
        requests: &OrdinarySpriteTextureRequests,
        records: &AdmittedContainerAnimations,
        constructor_schedule: Option<OrdinarySpriteLoadSchedule>,
        force_pow2_at_new_load: Option<bool>,
        animation_programs_retain_vector: Option<bool>,
    ) -> Result<(OrdinarySpriteTextureRegistry, Self), OrdinarySpriteScheduleRejection> {
        if records.source_identity != requests.source {
            return Err(OrdinarySpriteScheduleRejection::AnimationSourceMismatch);
        }
        let schedule = requests
            .load_schedule
            .or(constructor_schedule)
            .ok_or(OrdinarySpriteScheduleRejection::ConstructorLoadScheduleUnavailable)?;
        let mut registry = presented_registry.clone();
        let base = match (&requests.base, schedule) {
            (
                OrdinarySpriteTextureField::File(request),
                OrdinarySpriteLoadSchedule::LegacyDeferred,
            ) => OrdinarySpriteResourceBinding::DeferredBase(request.clone()),
            _ => register_field(
                &mut registry,
                resolver,
                &requests.base,
                force_pow2_at_new_load,
            )?,
        };
        // Native order: base, program initialization, all animation pairs,
        // then ordinary masking_texture outside the +1d3 gate.
        if schedule == OrdinarySpriteLoadSchedule::Eager && !records.records.is_empty() {
            match animation_programs_retain_vector {
                Some(true) => {}
                Some(false) => return Err(
                    OrdinarySpriteScheduleRejection::AnimationVectorClearedRequiresLifetimeRebuild,
                ),
                None => {
                    return Err(
                        OrdinarySpriteScheduleRejection::AnimationProgramInitializationUnavailable,
                    )
                }
            }
        }
        let mut animations = Vec::with_capacity(records.records.len());
        for (native_ordinal, record) in records.records.iter().enumerate() {
            let mut bind = |path: &str,
                            role|
             -> Result<
                OrdinarySpriteResourceBinding,
                OrdinarySpriteScheduleRejection,
            > {
                if path.is_empty() {
                    return Ok(OrdinarySpriteResourceBinding::Absent);
                }
                if schedule == OrdinarySpriteLoadSchedule::LegacyDeferred {
                    return Ok(
                        OrdinarySpriteResourceBinding::NotInitializedByDeferredSchedule {
                            native_path: path.to_owned(),
                        },
                    );
                }
                let file = resolver.resolve_texture_file(path).map_err(|_| {
                    OrdinarySpriteScheduleRejection::AnimationWinnerUnavailable {
                        role,
                        native_path: path.to_owned(),
                    }
                })?;
                let content = resolver
                    .admitted_content_key(&file.rel_path)
                    .ok_or_else(
                        || OrdinarySpriteScheduleRejection::AnimationWinnerUnavailable {
                            role,
                            native_path: path.to_owned(),
                        },
                    )?;
                let loaded = registry
                    .get_or_register_ordinary(
                        resolver,
                        OrdinaryTextureRequest {
                            role,
                            native_path: path,
                            content,
                        },
                        force_pow2_at_new_load,
                    )
                    .map_err(OrdinarySpriteScheduleRejection::Resource)?;
                Ok(OrdinarySpriteResourceBinding::Loaded(loaded))
            };
            animations.push(OrdinaryAnimationResourcePair {
                authored_ordinal: record.authored_ordinal,
                mask: bind(
                    &record.mask_file,
                    OrdinaryTextureRole::AnimationMask(native_ordinal),
                )?,
                animated: bind(
                    &record.texture_file,
                    OrdinaryTextureRole::AnimationTexture(native_ordinal),
                )?,
            });
        }
        let masking_texture = register_field(
            &mut registry,
            resolver,
            &requests.masking_texture,
            force_pow2_at_new_load,
        )?;
        Ok((
            registry,
            Self {
                requests: requests.clone(),
                schedule,
                base,
                masking_texture,
                animations,
            },
        ))
    }

    /// Mirror the reached CSpriteType+88 size getter 14222b550 -> 14222b2a0.
    /// The ordinary CIcon SetPosition path reaches this via 1422e8ee0 ->
    /// CSprite+158(1422e9530) -> 14222b4b0. Call before consuming dimensions
    /// for that operation, independently of whether the later draw is queued.
    /// On an already loaded type this returns the same immutable Arc; no new
    /// profile setting, resolution or filesystem read replaces that resource.
    pub(crate) fn stage_size_getter(
        &self,
        presented_registry: &OrdinarySpriteTextureRegistry,
        resolver: &AssetResolver,
        force_pow2_at_new_load: Option<bool>,
    ) -> Result<(OrdinarySpriteTextureRegistry, Self), OrdinarySpriteScheduleRejection> {
        let mut registry = presented_registry.clone();
        let mut next = self.clone();
        if let OrdinarySpriteResourceBinding::DeferredBase(request) = &self.base {
            let loaded = registry
                .get_or_register_ordinary(resolver, request.borrowed(), force_pow2_at_new_load)
                .map_err(OrdinarySpriteScheduleRejection::Resource)?;
            next.base = OrdinarySpriteResourceBinding::Loaded(loaded);
        }
        Ok((registry, next))
    }
}

fn register_field(
    registry: &mut OrdinarySpriteTextureRegistry,
    resolver: &AssetResolver,
    field: &OrdinarySpriteTextureField,
    force_pow2_at_new_load: Option<bool>,
) -> Result<OrdinarySpriteResourceBinding, OrdinarySpriteScheduleRejection> {
    Ok(match field {
        OrdinarySpriteTextureField::Absent => OrdinarySpriteResourceBinding::Absent,
        OrdinarySpriteTextureField::Empty { field_ordinal } => {
            OrdinarySpriteResourceBinding::AuthoredEmpty {
                field_ordinal: *field_ordinal,
            }
        }
        OrdinarySpriteTextureField::File(request) => OrdinarySpriteResourceBinding::Loaded(
            registry
                .get_or_register_ordinary(resolver, request.borrowed(), force_pow2_at_new_load)
                .map_err(OrdinarySpriteScheduleRejection::Resource)?,
        ),
    })
}
