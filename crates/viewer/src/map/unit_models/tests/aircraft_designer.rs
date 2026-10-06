use super::*;
use crate::map::raids::arrows::designer_entity;
use replay::{
    frame::{Metrics, TableView},
    rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, LexicalTokenRow, PoliticalPartyRow,
        PoliticsRow, SimulationFeatureRow, TableId, TableRows,
    },
};

const ARCHETYPE: u32 = 7;
const MODEL: u32 = 8;
const DLC: u32 = 0x100;

/// `transport_plane_equipment` is an archetype the `transport_plane`
/// air sub-unit needs, so it carries a sub-unit list;
/// `transport_plane_equipment_1` does not.
fn content() -> model_selection::Content {
    model_selection::Content::from_test_documents(&[br#"
                sub_units = {
                    transport_plane = { sprite = transport_plane need = { transport_plane_equipment = 1 } }
                }
                equipments = {
                    transport_plane_equipment = { is_archetype = yes type = air_transport sprite = transport_plane }
                    transport_plane_equipment_1 = { archetype = transport_plane_equipment }
                }
            "#])
}

fn assets(registered: &[&str]) -> UnitModelAssets {
    let names = equipment_graphics::Names {
        countries: ["ENG".to_owned()].into(),
        continents: BTreeSet::new(),
    };
    UnitModelAssets {
                sub_units: content(),
                graphics: equipment_graphics::Content::from_texts(
                    &[b"default = { transport_plane_equipment = { pool = { models = { pool_c47_entity } } } }"],
                    &names,
                )
                .unwrap(),
                registered: registered.iter().map(|name| (*name).to_owned()).collect(),
                disabled: None,
                ..UnitModelAssets::off(String::new())
            }
}

/// ENG (raw 100, slot 1) created variant 5 of `token`, saved model
/// `saved`; the DLC mask is `dlc`.
fn frame(token: u32, saved: &str, dlc: u32) -> DecodedFrame {
    let mut views = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed for this hour".into(),
    });
    let recorded = [
        (
            TableId::CountryLinks,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            }]),
        ),
        (
            TableId::Countries,
            TableRows::Countries(vec![CountryRow {
                id: 100,
                canonical_slot: 1,
                tag: b"ENG".to_vec(),
                ..Default::default()
            }]),
        ),
        (
            TableId::CountryAppearance,
            TableRows::CountryAppearance(vec![CountryAppearanceRow {
                country: 100,
                graphical_culture: b"western_european_gfx".to_vec(),
                ..Default::default()
            }]),
        ),
        (
            TableId::PoliticalParties,
            TableRows::PoliticalParties(vec![PoliticalPartyRow {
                country: 100,
                is_ruling: 1,
                token: b"democratic".to_vec(),
                tag: 100,
                ..Default::default()
            }]),
        ),
        (
            TableId::Politics,
            TableRows::Politics(vec![PoliticsRow {
                country: 100,
                ..Default::default()
            }]),
        ),
        (TableId::Provinces, TableRows::Provinces(Vec::new())),
        (TableId::Factions, TableRows::Factions(Vec::new())),
        (
            TableId::FactionMembers,
            TableRows::FactionMembers(Vec::new()),
        ),
        (TableId::States, TableRows::States(Vec::new())),
        (
            TableId::LexicalTokens,
            TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: ARCHETYPE,
                    literal: b"transport_plane_equipment".to_vec(),
                },
                LexicalTokenRow {
                    token: MODEL,
                    literal: b"transport_plane_equipment_1".to_vec(),
                },
            ]),
        ),
        (
            TableId::VariantDefinitions,
            TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: 5,
                equipment_token: token,
                archetype_token: ARCHETYPE,
                creator_link: 100,
                classifier_flags: 0x10_0000,
                explicit_entity: saved.as_bytes().to_vec(),
                ..Default::default()
            }]),
        ),
        (
            TableId::VariantUpgrades,
            TableRows::VariantUpgrades(Vec::new()),
        ),
        (
            TableId::AvailableEquipmentVariants,
            TableRows::AvailableEquipmentVariants(Vec::new()),
        ),
        (
            TableId::SimulationFeatures,
            TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: dlc,
                human_ai: 0,
                weather_suspend: replay::rows::Recorded::Value(None),
            }]),
        ),
    ];
    for (id, rows) in recorded {
        views[id.index()] = TableView::Complete(rows);
    }
    DecodedFrame {
        branch: 1,
        hour: 1,
        tables: views.map(Into::into),
        metrics: Metrics::default(),
    }
}

/// `designer_entity` for variant 5 as the raid layer asks it.
fn resolve(
    assets: &UnitModelAssets,
    token: u32,
    saved: &str,
    dlc: u32,
) -> Result<Option<String>, String> {
    let frame = frame(token, saved, dlc);
    let variant = VariantDefinitionRow {
        id: 5,
        equipment_token: token,
        archetype_token: ARCHETYPE,
        creator_link: 100,
        explicit_entity: saved.as_bytes().to_vec(),
        ..Default::default()
    };
    let equipment = if token == ARCHETYPE {
        "transport_plane_equipment"
    } else {
        "transport_plane_equipment_1"
    };
    designer_entity(
        &variant,
        equipment,
        assets.sub_units.equipment_content(),
        "ENG",
        dlc,
        &|| {
            equipment_graphics::tests::with_world(&frame, &[], |limits| {
                assets.aircraft_variant_model(&frame, limits, 5, 100)
            })
        },
        &|name| assets.declares(name),
    )
}

const ALL: [&str; 3] = [
    "pool_c47_entity",
    "saved_c47_entity",
    "ENG_transport_plane_entity",
];

#[test]
fn with_the_dlc_an_archetype_without_a_saved_model_takes_its_pool_model() {
    let assets = assets(&ALL);
    let recorded = frame(ARCHETYPE, "", DLC);
    assert_eq!(
        equipment_graphics::tests::with_world(&recorded, &[], |limits| {
            assets.aircraft_variant_model(&recorded, limits, 5, 100)
        }),
        Ok(Some("pool_c47_entity".into()))
    );
    assert_eq!(
        resolve(&assets, ARCHETYPE, "", DLC),
        Ok(Some("pool_c47_entity".into()))
    );
}

#[test]
fn a_saved_model_is_answered_before_the_pools_are_run() {
    // `ResolveEquipmentVariantModel@14191A580` returns a nonempty
    // `+0x430` without running `141918420`, so with the DLC the saved
    // model is the result even where a pool model is declared.
    let assets = assets(&ALL);
    assert_eq!(
        resolve(&assets, ARCHETYPE, "saved_c47_entity", DLC),
        Ok(Some("saved_c47_entity".into()))
    );
    // An undeclared saved model goes on to the aircraft ladder.
    assert_eq!(
        resolve(&assets, ARCHETYPE, "missing_entity", DLC),
        Ok(Some("ENG_transport_plane_entity".into()))
    );
}

#[test]
fn an_undeclared_pool_model_falls_through_to_the_aircraft_ladder() {
    // No pool or conventional candidate is declared: the result is
    // null, the empty `+0x430` gives nothing, and
    // `FindAircraftEquipmentEntity@141236950` answers.
    let assets = assets(&["ENG_transport_plane_entity"]);
    let recorded = frame(ARCHETYPE, "", DLC);
    assert_eq!(
        equipment_graphics::tests::with_world(&recorded, &[], |limits| {
            assets.aircraft_variant_model(&recorded, limits, 5, 100)
        }),
        Ok(None)
    );
    assert_eq!(
        resolve(&assets, ARCHETYPE, "", DLC),
        Ok(Some("ENG_transport_plane_entity".into()))
    );
}

#[test]
fn without_the_dlc_or_a_sub_unit_list_the_pools_are_not_run() {
    let assets = assets(&ALL);
    // No DLC: the saved model, then the ladder.
    assert_eq!(
        resolve(&assets, ARCHETYPE, "saved_c47_entity", 0),
        Ok(Some("saved_c47_entity".into()))
    );
    assert_eq!(
        resolve(&assets, ARCHETYPE, "", 0),
        Ok(Some("ENG_transport_plane_entity".into()))
    );
    // only its saved model.
    let recorded = frame(MODEL, "", DLC);
    assert_eq!(
        equipment_graphics::tests::with_world(&recorded, &[], |limits| {
            assets.aircraft_variant_model(&recorded, limits, 5, 100)
        }),
        Ok(None)
    );
    let recorded = frame(MODEL, "saved_c47_entity", DLC);
    assert_eq!(
        equipment_graphics::tests::with_world(&recorded, &[], |limits| {
            assets.aircraft_variant_model(&recorded, limits, 5, 100)
        }),
        Ok(Some("saved_c47_entity".into()))
    );
    assert_eq!(
        resolve(&assets, MODEL, "", DLC),
        Ok(Some("ENG_transport_plane_entity".into()))
    );
}
