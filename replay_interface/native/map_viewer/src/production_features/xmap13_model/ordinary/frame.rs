use super::*;
use crate::{GameFileResolver, Xmap12EntityCache, Xmap12ResolvedEntityChain};
use portable_content_manifest::ValidatedContentAdmission;
use std::sync::OnceLock;
use std::cell::RefCell;

pub(crate) struct OrdinaryFrame<'a> {
    world: &'a replay_core::WorldFrame,
    resolver: &'a GameFileResolver,
    admission: &'a ValidatedContentAdmission,
    model_viewer_country: Option<&'a str>,
    assets: OnceLock<std::result::Result<FrameAssets, String>>,
    label_issues: RefCell<Vec<label::LabelIssue>>,
}

struct FrameAssets {
    native: NativeDesignerInputs,
    equipment: AuthoredEquipmentCatalog,
    subunits: BTreeMap<String, SubunitModel>,
    pools: OnceLock<std::result::Result<pools::ModelPools, String>>,
    labels: OnceLock<std::result::Result<label::ModelLabelCatalog, String>>,
}

impl<'a> OrdinaryFrame<'a> {
    pub(crate) fn new(world: &'a replay_core::WorldFrame, resolver: &'a GameFileResolver,
        admission: &'a ValidatedContentAdmission, model_viewer_country: Option<&'a str>) -> Self {
        Self { world, resolver, admission, model_viewer_country, assets: OnceLock::new(),
            label_issues: RefCell::new(Vec::new()) }
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
        Ok(FrameAssets { native, equipment, subunits, pools: OnceLock::new(), labels: OnceLock::new() })
    }

    fn localized(&self, value: label::LocalizedLabel) -> Result<String> {
        let mut issues = self.label_issues.borrow_mut();
        for issue in value.issues {
            if !issues.contains(&issue) { issues.try_reserve(1)?; issues.push(issue); }
        }
        Ok(value.text)
    }

    pub(crate) fn take_label_diagnostics(&self) -> Result<Vec<(&'static str, String)>> {
        let issues = std::mem::take(&mut *self.label_issues.borrow_mut());
        let mut output = Vec::new();
        output.try_reserve(issues.len())?;
        for issue in issues {
            let cause = match &issue {
                label::LabelIssue::TopLevelMiss(_) => "model-label-localisation-key-missing",
                label::LabelIssue::NestedMiss(_) => "model-label-nested-localisation-key-missing",
                label::LabelIssue::DepthLimit => "model-label-format-depth-limit",
                label::LabelIssue::ByteLimit => "model-label-format-byte-limit",
                label::LabelIssue::UnmatchedDollar => "model-label-unmatched-variable-delimiter",
            };
            output.push((cause, format!("XMAP13 ordinary model label: {issue:?}; native formatter output retained; this content diagnostic does not change the selected model")));
        }
        Ok(output)
    }

    fn label_catalog<'b>(&self, assets: &'b FrameAssets) -> Result<&'b label::ModelLabelCatalog> {
        assets.labels.get_or_init(|| label::ModelLabelCatalog::load(self.resolver, self.admission)
            .map_err(|error| error.to_string())).as_ref().map_err(|reason| reason.clone().into())
    }

    fn plain_label(&self, assets: &FrameAssets, key: &str) -> Result<String> {
        self.localized(self.label_catalog(assets)?.plain(self.resolver, self.admission, key)?)
    }

    fn country_label(&self, assets: &FrameAssets, tag: &str, key: &str) -> Result<String> {
        self.localized(self.label_catalog(assets)?.country(self.resolver, self.admission, tag, key)?)
    }

    fn variant_label(&self, assets: &FrameAssets, variant: &NativeDesignerVariantInputs) -> Result<String> {
        label::variant_model_label(variant, || {
            let country = if let Some(origin) = &variant.model.origin_override_tag {
                // A27's positive raw origin operand is already projected through the TAG table.
                // It does not request that country's effective naming override a second time.
                origin.clone()
            } else {
                // Reuse the current base-country naming envelope only after its exact identity
                // join. A27's country-table physical index is not the creator's raw TAG id.
                self.world.country_identity_for_tag(&variant.model.creator_tag)?;
                match self.world.country_name_semantics(&variant.model.creator_tag).effective_name_tag {
                    replay_core::country_name_semantics::NamingInput::Measured(name) => name.tag,
                    replay_core::country_name_semantics::NamingInput::Absent =>
                        return Err("XMAP13 model label creator effective name is absent".into()),
                    replay_core::country_name_semantics::NamingInput::Unavailable(reason) =>
                        return Err(format!("XMAP13 model label creator effective name: {reason:?}").into()),
                }
            };
            self.country_label(assets, &country, &variant.equipment_definition_token)
        })
    }

    fn row_label(&self, assets: &FrameAssets, source: ModelLabelSource<'_>) -> Result<String> {
        use snapshot::design_catalog::DesignTextObservation;
        match source {
            ModelLabelSource::GraphicArchetype(key) => self.plain_label(assets, key),
            ModelLabelSource::GenericSubunit(key) => {
                let viewer = self.model_viewer_country
                    .ok_or("XMAP13 generic model label requires current viewer country")?;
                self.world.country_identity_for_tag(viewer)?;
                self.country_label(assets, viewer, key)
            }
            ModelLabelSource::Variant { row, override_label } => {
                if let Some(label) = override_label { return Ok(label.to_owned()); }
                let display = match &row.display_name {
                    DesignTextObservation::Measured { value, .. } => value,
                    DesignTextObservation::Unavailable { reason } =>
                        return Err(format!("XMAP13 suffix model display name: {reason:?}").into()),
                };
                if display.is_empty() {
                    let definition = assets.equipment.definition(&row.equipment_definition_token)?;
                    for derived in [true, false] {
                        if let Some((key, source)) = definition.model_label_key(derived)? {
                            if self.admission.require_used_content(&source.normalized_game_relative_path)? != source {
                                return Err("XMAP13 authored model label source disagrees with admission".into());
                            }
                            if !key.is_empty() {
                                return self.country_label(assets, &row.model.creator_tag, key);
                            }
                        }
                    }
                }
                self.variant_label(assets, row)
            }
        }
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

    fn template_entity(&self, assets: &FrameAssets, input: &DivisionModelInputs,
        composition: &DivisionComposition, cache: &mut Xmap12EntityCache,
    ) -> Result<Option<Xmap12ResolvedEntityChain>> {
        use snapshot::design_catalog::{DesignI32Observation, DesignTextObservation};
        use template_queue::{ModelRow, TemplateQueues};
        let default_label = self.plain_label(assets, "USE_DEFAULT_MODEL")?;
        let viewer = self.model_viewer_country
            .ok_or("XMAP13 template fallback requires current viewer country")?;
        let viewer_handle = self.world.country_identity_for_tag(viewer)?;
        let mut variants = country_variants(&assets.native, viewer, VariantSourceList::Combined)?;
        sort_variants(&mut variants, viewer_handle, self.world, &assets.equipment, &assets.native.runtime_dlc_mask)?;
        let preference = input.template_preference.measured()
            .ok_or("XMAP13 template model preference is unavailable")?;
        let (_, primary) = primary_subunit(composition, &assets.subunits)?;
        let selected_mask = match preference.category { Some(mask) => mask, None => primary.type_mask()? };
        let strict = preference.category.is_some();
        let labeler = |source: ModelLabelSource<'_>| self.row_label(assets, source);
        let mut queues = TemplateQueues::default();
        // These are persistent native suffix queues. A failed duplicate insertion changes
        // preferred/alternate flow, so they cannot be rebuilt and deduplicated afterward.
        let mut suffix: [ModelList<'_, Xmap12ResolvedEntityChain>; 3] =
            std::array::from_fn(|_| ModelList { labeler: Some(&labeler), ..ModelList::default() });
        let mut resolve = |token: &str| crate::xmap12_population::entity_chain(
            self.resolver, self.admission, cache, token).map(|row| row.cloned());
        let scalar = |field: &DesignI32Observation, name: &str| -> Result<i32> {
            match field {
                DesignI32Observation::Measured { raw_i32 } => Ok(*raw_i32),
                DesignI32Observation::Unavailable { reason } =>
                    Err(format!("XMAP13 template {name}: {reason:?}").into()),
            }
        };
        for (subunit_name, subunit, count) in counted_subunits(composition, &assets.subunits)?.0 {
            let count100 = count.wrapping_mul(100);
            for variant in &variants {
                let excluded = preference.excluded_variants.measured()
                    .ok_or("XMAP13 template variant exclusions are unavailable")?;
                let id = variant_id(variant)?;
                if excluded.iter().flatten().any(|excluded| *excluded == id) { continue; }
                let archetype = assets.equipment.archetype_identity(&variant.equipment_definition_token)?;
                let needs = subunit.need_pool(&assets.equipment)?;
                let amount = needs.iter().find(|need| Some(need.archetype.as_str())
                    == archetype.as_ref().map(|row| row.token.as_str())).map(|need| need.amount_raw).unwrap_or(0);
                if amount <= 0 { continue; }
                let mask = subunit.type_mask()?;
                let mut templates = assets.native.templates.iter().filter(|row| row.template_id == preference.template_id);
                let template = templates.next().ok_or("XMAP13 same-hour template country inputs are absent")?;
                if templates.next().is_some() || template.design_id != format!("{}:52", preference.template_id) {
                    return Err("XMAP13 template country input identity is contended".into());
                }
                // 141919643 performs this lookup before the conditional creator override.
                let initial = scalar(&template.country_289a, "country_289a")?;
                let initial_country = self.world.design_country_identity_for_handle(initial)?;
                let mut country = initial_country.tag().to_owned();
                if creator_override_allowed(mask, || variant_category(&assets.equipment, variant))? {
                    let creator = variant.model.origin_override_tag.as_deref().unwrap_or(&variant.model.creator_tag);
                    let creator_handle = self.world.country_identity_for_tag(creator)?;
                    if (creator_handle as i32) > 0 {
                        country = self.world.design_country_identity_for_handle(creator_handle as i32)?.tag().to_owned();
                    }
                }
                let naming = self.world.country_name_semantics(&country);
                let tag = replay_core::country_name_semantics::NamingInput::Measured(country.clone());
                let context = SuffixContext { native: &assets.native, world: self.world, equipment: &assets.equipment,
                    country: &country, prefixes: [&naming.cosmetic_name_base, &tag, &naming.graphical_culture] };
                let selected = AllocatedModel { variant, subunit_name, subunit };
                let override_label = self.variant_label(assets, variant)?;
                let pools = assets.pools.get_or_init(|| pools::ModelPools::load(self.resolver, self.admission)
                    .map_err(|error| error.to_string())).as_ref().map_err(Clone::clone)?
                    .checked_rows(self.resolver, self.admission, &assets.equipment, self.world, &country,
                        variant, subunit_name, &assets.native.runtime_dlc_mask)?;
                let rows = complete_model_list(&selected, &context, &pools, default_label.clone(),
                    override_label, &labeler, &mut resolve)?;
                let normal = subunit.visual_priority.unwrap_or_else(|| count100.wrapping_add(10));
                let normal = match subunit.visual_priority {
                    Some(_) => normal,
                    None => normal.wrapping_add(assets.equipment.effective_model_priority(&variant.equipment_definition_token)?).wrapping_mul(10),
                };
                let explicit = subunit.visual_priority.unwrap_or_else(|| count100.wrapping_add(110).wrapping_mul(10));
                for outer in template_queue::outer_bits(selected_mask) {
                    let reached = (mask as u32) != 0 && (!strict || (mask as u32) & outer != 0);
                    let explicit_token = if reached {
                        match &variant.model.explicit_entity {
                            DesignTextObservation::Measured { value, .. } => value.as_str(),
                            DesignTextObservation::Unavailable { reason } =>
                                return Err(format!("XMAP13 template variant explicit entity: {reason:?}").into()),
                        }
                    } else { "" };
                    queues.admit_variant_for_bit(outer, mask, strict, subunit.visual_priority,
                        normal, explicit, explicit_token, &rows)?;
                    let alternative = scalar(&template.country_3abe, "country_3abe")?;
                    let suffix_handle = if alternative > 0 { alternative } else { initial };
                    if let Some(bucket) = template_queue::suffix_bucket(outer, mask, strict, subunit.visual_priority.is_some()) {
                        let suffix_country = self.world.design_country_identity_for_handle(suffix_handle)?;
                        let suffix_country = suffix_country.tag();
                        let naming = self.world.country_name_semantics(suffix_country);
                        let tag = replay_core::country_name_semantics::NamingInput::Measured(suffix_country.to_owned());
                        let context = SuffixContext { native: &assets.native, world: self.world, equipment: &assets.equipment,
                            country: suffix_country, prefixes: [&naming.cosmetic_name_base, &tag, &naming.graphical_culture] };
                        suffix[bucket].suffix_families(subunit_name, subunit, None, normal, &context, &mut resolve)?;
                    }
                }
            }
        }
        let [first, second, third] = suffix;
        let convert = |models: ModelList<'_, Xmap12ResolvedEntityChain>| -> Result<Vec<ModelRow<Xmap12ResolvedEntityChain>>> {
            let mut rows = Vec::new();
            rows.try_reserve(models.rows.len())?;
            for row in models.rows { rows.push(ModelRow { label: row.label, token: row.token,
                payload_raw: row.weight_raw, entity: Some(row.entity) }); }
            Ok(rows)
        };
        let mut rows = queues.finish(ModelRow { label: default_label, token: String::new(),
            payload_raw: -100_000, entity: None }, [convert(first)?, convert(second)?, convert(third)?])?;
        label::order_by_native_label_rank(&mut rows, |row| &row.label)?;
        label::decorate_model_labels(&mut rows, |row| &mut row.label)?;
        // 141918180 consumes physical row one, never scans later rows for a valid entity.
        Ok(rows.into_iter().nth(1).and_then(|row| row.entity))
    }

    pub(crate) fn entity(&self, ordinal: usize, input: &DivisionModelInputs,
        composition: &DivisionComposition, cache: &mut Xmap12EntityCache,
    ) -> Result<Option<Xmap12ResolvedEntityChain>> {
        let assets = self.assets.get_or_init(|| self.load().map_err(|error| error.to_string()))
            .as_ref().map_err(|reason| reason.clone())?;
        let Some(selected) = allocated_model(input, composition, &assets.subunits, &assets.equipment, &assets.native)? else {
            if let Some(entity) = self.template_entity(assets, input, composition, cache)? { return Ok(Some(entity)); }
            let (name, primary) = primary_subunit(composition, &assets.subunits)?;
            let unit = self.world.division_units().get(ordinal).ok_or("XMAP13 ordinary division ordinal is unavailable")?;
            let country = unit.owner.measured().ok_or("XMAP13 terminal model fallback owner is unavailable")?.tag();
            self.world.country_identity_for_tag(country)?;
            return self.subunit_fallback(ordinal, country, name, primary, cache);
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
        self.subunit_fallback(ordinal, &country, selected.subunit_name, selected.subunit, cache)
    }

    fn subunit_fallback(&self, ordinal: usize, country: &str, subunit_name: &str, subunit: &SubunitModel,
        cache: &mut Xmap12EntityCache,
    ) -> Result<Option<Xmap12ResolvedEntityChain>> {
        use replay_core::division_retained_visual_variant::{
            DivisionRetainedVisualVariantRefusal as Refusal, RetainedVisualVariantObservation as Observation,
        };
        let unit = self.world.division_units().get(ordinal)
            .ok_or("XMAP13 terminal model fallback division ordinal is unavailable")?;
        let kind = unit.raw_division_type.measured().copied()
            .ok_or("XMAP13 terminal model fallback division type is unavailable")?;
        // The admitted CArmy's primary vtable +0x40 returns the identical primary
        // address. Its selector owner +0x1d8 is collector body +0x1c8. The accessor
        // separately checks this acquisition's owner witness and current mapping.
        let retained = match self.world.division_retained_visual_variant(unit.raw_division_id, kind) {
            Ok(Observation::Measured(raw)) => Some(raw),
            Ok(Observation::Unavailable(_)) | Err(Refusal::Missing | Refusal::Unobtained) => None,
            Err(reason) => return Err(format!("XMAP13 terminal model fallback retained input: {reason:?}").into()),
        };
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
            Leaf::Measured(country.to_owned()), prefix("graphical_culture", naming.graphical_culture)?];
        let mut exists = |token: &str| {
            crate::xmap12_population::entity_chain(self.resolver,self.admission,cache,token)
                .map(|chain| chain.is_some())
        };
        let token = match retained {
            Some(raw) => subunit_entity(subunit_name, &subunit.sprite, raw, prefixes.each_ref(), &mut exists),
            None => subunit_entity_without_retained_variant(subunit_name, &subunit.sprite,
                prefixes.each_ref(), &mut exists),
        }?;
        let Some(token) = token else { return Ok(None); };
        crate::xmap12_population::entity_chain(self.resolver,self.admission,cache,&token)
            .map(|chain| chain.cloned())
    }
}
