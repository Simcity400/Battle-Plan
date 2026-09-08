use super::*;
use crate::{GameFileResolver, Xmap12EntityCache, Xmap12ResolvedEntityChain};
use portable_content_manifest::ValidatedContentAdmission;
use std::sync::OnceLock;

pub(crate) struct OrdinaryFrame<'a> {
    world: &'a replay_core::WorldFrame,
    resolver: &'a GameFileResolver,
    admission: &'a ValidatedContentAdmission,
    model_viewer_country: Option<&'a str>,
    assets: OnceLock<std::result::Result<FrameAssets, String>>,
}

struct FrameAssets {
    native: NativeDesignerInputs,
    equipment: AuthoredEquipmentCatalog,
    subunits: BTreeMap<String, SubunitModel>,
    pools: OnceLock<std::result::Result<pools::ModelPools, String>>,
}

impl<'a> OrdinaryFrame<'a> {
    pub(crate) fn new(world: &'a replay_core::WorldFrame, resolver: &'a GameFileResolver,
        admission: &'a ValidatedContentAdmission, model_viewer_country: Option<&'a str>) -> Self {
        Self { world, resolver, admission, model_viewer_country, assets: OnceLock::new() }
    }

    fn load(&self) -> Result<FrameAssets> {
        let lease = self.resolver.content_admission_lease().ok_or("XMAP13 ordinary models require content admission")?;
        if lease.manifest_digest_sha256() != Some(self.admission.manifest_digest_sha256()) {
            return Err("XMAP13 ordinary model resolver and admission disagree".into());
        }
        let native = self.world.design_catalog().ok_or("XMAP13 ordinary model A27 catalog is unavailable")?
            .native_designer_inputs_at_hour(u64::try_from(self.world.hour_index())?)?;
        if native.hour != u64::try_from(self.world.hour_index())? {
            return Err("XMAP13 ordinary model A27 catalog belongs to another hour".into());
        }
        let equipment = AuthoredEquipmentCatalog::load(self.resolver)?;
        let equipment = match crate::ActiveDlcPresentation::from_session_metadata(self.world.session_metadata()) {
            Ok(active) => equipment.with_active_dlcs(&active),
            Err(_) => equipment,
        }.resolve_generated()?;
        let (subunits, _) = admitted_subunits(self.resolver)?;
        Ok(FrameAssets { native, equipment, subunits, pools: OnceLock::new() })
    }

    fn helper_country(&self, ordinal: usize, selected: &AllocatedModel<'_>,
        equipment: &AuthoredEquipmentCatalog,
    ) -> Result<String> {
        let unit = self.world.division_units().get(ordinal).ok_or("XMAP13 ordinary division ordinal is unavailable")?;
        let owner = unit.owner.measured().ok_or("XMAP13 ordinary model owner is unavailable")?;
        let owner = owner.tag();
        self.world.country_identity_for_tag(owner)?;
        let override_country = creator_override_allowed(selected.subunit.type_mask()?,
            || variant_category(equipment, selected.variant))?;
        let country = if override_country {
            selected.variant.model.origin_override_tag.as_deref().unwrap_or(&selected.variant.model.creator_tag)
        } else { owner };
        self.world.country_identity_for_tag(country)?;
        Ok(country.to_owned())
    }

    pub(crate) fn entity(&self, ordinal: usize, input: &DivisionModelInputs,
        composition: &DivisionComposition, cache: &mut Xmap12EntityCache,
    ) -> Result<Option<Xmap12ResolvedEntityChain>> {
        let assets = self.assets.get_or_init(|| self.load().map_err(|error| error.to_string()))
            .as_ref().map_err(|reason| reason.clone())?;
        let Some(selected) = allocated_model(input, composition, &assets.subunits, &assets.equipment, &assets.native)? else {
            let viewer = self.model_viewer_country
                .ok_or("XMAP13 template fallback viewer country is unavailable in this derivation context")?;
            self.world.country_identity_for_tag(viewer)?;
            return Err("XMAP13 template fallback requires independent template-country operands and the exact model-label inputs".into());
        };
        let selected_entity = allocated_entity(&selected,
            &mut |token| crate::xmap12_population::entity_chain(self.resolver,self.admission,cache,token)
                .map(|chain| chain.cloned()),
            |resolve| {
                let country = self.helper_country(ordinal, &selected, &assets.equipment)?;
                let naming = self.world.country_name_semantics(&country);
                let tag = replay_core::country_name_semantics::NamingInput::Measured(country.clone());
                let context = SuffixContext { native:&assets.native, world:self.world, equipment:&assets.equipment,
                    country:&country, prefixes:[&naming.cosmetic_name_base,&tag,&naming.graphical_culture] };
                allocated_model_list(&selected,&context,|| {
                    assets.pools.get_or_init(|| pools::ModelPools::load(self.resolver,self.admission)
                        .map_err(|error| error.to_string())).as_ref().map_err(|reason| reason.clone())?
                        .checked_rows(self.resolver,self.admission,&assets.equipment,self.world,&country,
                            selected.variant,selected.subunit_name,&assets.native.runtime_dlc_mask)
                },|| alternate::selected(&assets.native.runtime_dlc_mask),resolve)
            })?;
        if selected_entity.is_some() { return Ok(selected_entity); }
        // 141918180 recomputes 14191a770's country before its 141917d00 fallback.
        let country = self.helper_country(ordinal, &selected, &assets.equipment)?;
        let naming = self.world.country_name_semantics(&country);
        let prefix = |field, value| -> Result<Leaf<String>> {
            use replay_core::country_name_semantics::NamingInput;
            match value {
                NamingInput::Measured(value) => Ok(Leaf::Measured(value)),
                NamingInput::Absent => Ok(Leaf::Absent),
                NamingInput::Unavailable(reason) => Err(format!("XMAP13 model fallback {field}: {reason:?}").into()),
            }
        };
        let prefixes = [prefix("cosmetic_name_base", naming.cosmetic_name_base)?,
            Leaf::Measured(country), prefix("graphical_culture", naming.graphical_culture)?];
        let token = subunit_entity_without_retained_variant(selected.subunit_name, &selected.subunit.sprite,
            prefixes.each_ref(), &mut |token| {
                crate::xmap12_population::entity_chain(self.resolver,self.admission,cache,token)
                    .map(|chain| chain.is_some())
            })?;
        let Some(token) = token else { return Ok(None); };
        crate::xmap12_population::entity_chain(self.resolver,self.admission,cache,&token)
            .map(|chain| chain.cloned())
    }
}
