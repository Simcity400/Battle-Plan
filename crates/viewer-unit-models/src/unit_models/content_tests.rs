
use super::*;

fn roster(source: &[u8]) -> mesh::Roster {
    mesh::Roster {
        descriptors: mesh::parse_descriptors(source, "gfx/entities/known.asset").unwrap().into(),
        unreadable_names: BTreeMap::from([("uncertain".into(), "gfx/entities/later.asset".into())]),
        ..Default::default()
    }
}

#[test]
fn uncertain_attached_entity_refuses_a_cached_known_root() {
    let assets = UnitModelAssets {
        roster: roster(
            br#"
            entity = { name = root attach = { mount = child } }
            entity = { name = child attach = { muzzle = uncertain } }
            entity = { name = uncertain }
        "#,
        )
        .into(),
        registered: ["root".into()].into(),
        ..UnitModelAssets::off(String::new())
    };
    assets
        .library
        .write()
        .unwrap()
        .index
        .insert("root".into(), 0);
    let reason = entity_refusal(&assets.roster, "root").unwrap();
    assert!(reason.contains("root reaches uncertain"), "{reason}");
    assert!(reason.contains("gfx/entities/later.asset"), "{reason}");
    assert_eq!(assets.bake_slot("root"), None);
}

#[test]
fn uncertainty_traversal_uses_effective_attachments_and_bounds_cycles() {
    let roster = roster(
        br#"
        entity = { name = base attach = { name = mount node = uncertain } }
        entity = { name = replacement clone = base attach = { name = mount node = known } }
        entity = { name = known }
        entity = { name = cycle_a attach = { mount = cycle_b } }
        entity = { name = cycle_b attach = { mount = cycle_a } }
        entity = { name = uncertain }
    "#,
    );
    assert!(entity_refusal(&roster, "base").is_some());
    assert_eq!(entity_refusal(&roster, "replacement"), None);
    assert_eq!(entity_refusal(&roster, "cycle_a"), None);
    assert_eq!(entity_refusal(&roster, "known"), None);
}
