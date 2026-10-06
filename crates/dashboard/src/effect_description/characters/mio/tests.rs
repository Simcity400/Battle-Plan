use super::*;
use crate::{
    effect_description::tests::Fixture,
    scripted_text::{EventObject, EventScope},
};

fn assert_carrier_refusal(body: &str, class: &str, key: &str) {
    let fixture = Fixture::new();
    let mut graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
    let out = fixture.render(body, &mut graph, Content::default());
    assert_eq!(out.text, "");
    assert_eq!(
        out.refusals,
        BTreeMap::from([(
            format!(
                "synthetic effect {class}: key {key}: CEventScope INDUSTRIAL_ORGANISATION_REFERENCE (+0x78/+0x7C) is not represented by the event graph (140529390); IndustrialOrganisations records the handles"
            ),
            1
        )])
    );
}

// common/characters/NOR.txt:639. Scope switch mio:... belongs to the core;
// this is the reached block operand at the class's own description boundary.
#[test]
fn unlock_mio_trait_tooltip() {
    assert_carrier_refusal(
        "unlock_mio_trait_tooltip={trait=synthetic_convoy_production show_modifiers=yes}",
        "NIndustrialOrganisation::CTraitTooltipEffect",
        "unlock_mio_trait_tooltip",
    );
}

// common/national_focus/bulgaria.txt:1356: scalar under every MIO.
#[test]
fn complete_mio_trait() {
    assert_carrier_refusal(
        "complete_mio_trait=synthetic_prussia_trait",
        "NIndustrialOrganisation::CUnlockTraitEffect",
        "complete_mio_trait",
    );
}
