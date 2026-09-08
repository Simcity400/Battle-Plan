use super::*;
use replay_core::country_name_semantics::NamingInput;
use snapshot::design_catalog::{DesignTextObservation, DesignU32Observation};

pub(super) fn selected(runtime: &DesignU32Observation) -> Result<bool> {
    match runtime {
        DesignU32Observation::Measured { raw_u32 } => Ok(raw_u32 & (1 << 8) == 0),
        DesignU32Observation::Unavailable { reason } =>
            Err(format!("XMAP13 alternate equipment feature 0x26: {reason:?}").into()),
    }
}

pub(super) fn entity<T>(selected: &AllocatedModel<'_>, context: &SuffixContext<'_>,
    resolve: &mut impl FnMut(&str) -> Result<Option<T>>) -> Result<Option<T>> {
    match &selected.variant.model.explicit_entity {
        DesignTextObservation::Measured { value, .. } if !value.is_empty() => {
            if let Some(entity) = resolve(value)? { return Ok(Some(entity)); }
        },
        DesignTextObservation::Measured { .. } => {},
        DesignTextObservation::Unavailable { reason } =>
            return Err(format!("XMAP13 alternate variant explicit entity: {reason:?}").into()),
    }
    // 141918f90 supplies variant+1c, independently of the model-list helper country.
    let creator = &selected.variant.model.creator_tag;
    context.world.country_identity_for_tag(creator)?;
    let naming = context.world.country_name_semantics(creator);
    equipment_entity(context.equipment, &selected.variant.equipment_definition_token,
        &naming.cosmetic_name_base, creator, resolve)
}

fn equipment_entity<T>(equipment: &AuthoredEquipmentCatalog, definition: &str,
    cosmetic: &NamingInput<String>, tag: &str,
    resolve: &mut impl FnMut(&str) -> Result<Option<T>>) -> Result<Option<T>> {
    let cosmetic = match cosmetic {
        NamingInput::Measured(value) => value.as_str(),
        NamingInput::Absent => "",
        NamingInput::Unavailable(reason) =>
            return Err(format!("XMAP13 alternate equipment cosmetic name: {reason:?}").into()),
    };
    for (ordinal, prefix) in [Some(cosmetic), Some(tag), None].into_iter().enumerate() {
        if ordinal == 0 && cosmetic.is_empty() { continue; }
        let token = |name: &str| match prefix {
            Some(prefix) => format!("{prefix}_{name}{MODEL_ENTITY_SUFFIX}"),
            None => format!("{name}{MODEL_ENTITY_SUFFIX}"),
        };
        if let Some(entity) = resolve(&token(definition))? { return Ok(Some(entity)); }
        let archetype = equipment.archetype_identity(definition)?
            .ok_or("XMAP13 alternate equipment null-archetype name is unavailable")?;
        if let Some(entity) = resolve(&token(&archetype.token))? { return Ok(Some(entity)); }
        let sprite = equipment.definition(definition)?.model_sprite()?;
        if let Some(entity) = resolve(&token(sprite))? { return Ok(Some(entity)); }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_query_uses_bit_eight_and_preserves_unavailable() {
        for (raw, expected) in [(0,true),(1,true),(128,true),(256,false),(u32::MAX,false)] {
            assert_eq!(selected(&DesignU32Observation::Measured {raw_u32:raw}).unwrap(),expected);
        }
        assert!(selected(&DesignU32Observation::Unavailable { reason:snapshot::design_catalog::DesignFieldRefusal {
            kind:"unreadable".into(),field:"runtime_dlc_mask".into(),
        }}).is_err());
    }

    #[test]
    fn alternate_equipment_preserves_prefix_family_order_and_lazy_sprite() {
        let root=crate::test_support::temp_test_dir("xmap13-alternate-equipment");
        crate::test_support::write_modvfs_fixture(&root,"common/units/equipment/alternate.txt",
            b"equipments={base={is_archetype=yes sprite=inherited} child={archetype=base} bad={sprite={}}}");
        let raw=crate::GameFileResolver::vanilla(&root).unwrap();
        let paths=["common/units/equipment/alternate.txt".into()].into_iter().collect();
        let (resolver,admission)=crate::test_support::admitted_vanilla_resolver(&raw,&paths,"xmap13-alternate-equipment");
        let equipment=AuthoredEquipmentCatalog::load(&resolver).unwrap().resolve_generated().unwrap();
        let order=["COS_child_entity","COS_base_entity","COS_inherited_entity",
            "GER_child_entity","GER_base_entity","GER_inherited_entity",
            "child_entity","base_entity","inherited_entity"];
        for winner in 0..order.len() {
            let mut visited=Vec::new();
            let found=equipment_entity(&equipment,"child",&NamingInput::Measured("COS".into()),"GER",&mut |name| {
                visited.push(name.to_owned()); Ok((name==order[winner]).then(||name.to_owned()))
            }).unwrap();
            assert_eq!(found.as_deref(),Some(order[winner]));
            assert_eq!(visited,order[..=winner]);
        }
        let mut visited=Vec::new();
        assert!(equipment_entity(&equipment,"child",&NamingInput::Measured(String::new()),"GER",&mut |name| {
            visited.push(name.to_owned()); Ok::<Option<()>,_>(None)
        }).unwrap().is_none());
        assert_eq!(visited,order[3..]);
        assert_eq!(equipment_entity(&equipment,"bad",&NamingInput::Measured(String::new()),"GER",
            &mut |name| Ok(Some(name.to_owned()))).unwrap().as_deref(),Some("GER_bad_entity"));
        assert!(equipment_entity(&equipment,"bad",&NamingInput::Measured(String::new()),"GER",
            &mut |_| Ok::<Option<()>,_>(None)).is_err());
        drop((equipment,resolver,raw,admission));
        portable_test_fs_retry::remove_dir_all(root).unwrap();
    }
}
