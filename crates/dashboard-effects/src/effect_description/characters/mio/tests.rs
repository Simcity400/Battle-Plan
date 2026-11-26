use super::*;
use crate::{
    fixtures::{Folder, folder_catalogue},
    effect_description::tests::Fixture,
    scripted_text::{EventObject, EventScope, Link},
};
use replay_data::rows::*;
use replay_frame::frame::TableView;
use crate::fixtures::game;

const MIO: u64 = 79 | 1 << 32;
const EFFECTS: [(&str, &str, &str); 2] = [
    (
        "fixture_show_organisation_trait",
        "MIO trait tooltip",
        "Unlock Owner trait AAA\n",
    ),
    (
        "fixture_finish_organisation_trait",
        "MIO trait completion",
        "Complete Owner trait AAA\n",
    ),
];

fn fixture() -> (Fixture, Folder) {
    let mut fixture = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}",
        ),
        (
            "common/military_industrial_organization/organizations/test.txt",
            "synthetic_mio={trait={token=synthetic_convoy_production name=SYN_TRAIT equipment_bonus={soft_attack=0.14 supply_consumption=-0.07 reliability=0} organization_modifier={political_power_gain=0.23 custom_modifier_tooltip=RAW_BODY}} trait={token=synthetic_empty_trait} trait={token=production_gap name=SYN_TRAIT equipment_bonus={soft_attack=0.14} production_bonus={production_capacity_factor=0.30006 production_cost_factor=-0.00001} organization_modifier={political_power_gain=0.23 custom_modifier_tooltip=RAW_BODY}} trait={token=numeric_zero name=SYN_TRAIT equipment_bonus={soft_attack=0.14} organization_modifier={political_power_gain=0}} initial_trait={name=INITIAL}} inherited_mio={include=synthetic_mio}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n AAA:0 \"Alpha\"\n",
                " EFFECT_MIO_UNLOCK_TRAIT:0 \"Unlock $TRAIT$ [Root.GetTag]\"\n",
                " EFFECT_MIO_COMPLETE_TRAIT:0 \"Complete $TRAIT$ [Root.GetTag]\"\n",
                " EFFECT_MIO_UNLOCK_POLICY:0 \"Policy $NAME$\"\n",
                " SYN_TRAIT:0 \"Base trait\"\n AAA_SYN_TRAIT:0 \"Owner trait\"\n",
                " synthetic_mio_synthetic_empty_trait:0 \"Unmodified trait\"\n",
                " STAT_SYNTHETIC_SOFT_ATTACK:0 \"Soft\"\n STAT_SYNTHETIC_SUPPLY_CONSUMPTION:0 \"Supply\"\n",
                " STAT_MODIFIER_DIFF:0 \"+$VALUE|2$\"\n STAT_MODIFIER_DIFF_NEG:0 \"-$VALUE|2$\"\n",
                " PP:0 \"Power\"\n RAW_BODY:0 \"Raw [Root.GetTag]\"\n",
                " BONUS_PRODUCTION_CAPACITY_FACTOR:0 \"Capacity\"\n",
                " BONUS_PRODUCTION_COST_FACTOR:0 \"Cost\"\n",
                " unknown_trait:0 \"Must not localise missing trait\"\n"
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    fixture.catalogue = catalogue;
    *fixture.frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                organisation: MIO,
                country: 100,
                owner: 100,
                token: 10,
                ..Default::default()
            },
        ]));
    *fixture.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"synthetic_mio".to_vec(),
            },
            LexicalTokenRow {
                token: 11,
                literal: b"synthetic_convoy_production".to_vec(),
            },
            LexicalTokenRow {
                token: 12,
                literal: b"synthetic_empty_trait".to_vec(),
            },
            LexicalTokenRow {
                token: 13,
                literal: b"production_gap".to_vec(),
            },
            LexicalTokenRow {
                token: 14,
                literal: b"numeric_zero".to_vec(),
            },
            LexicalTokenRow {
                token: 31,
                literal: b"political_power_gain".to_vec(),
            },
        ]));
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            ModifierDefinitionRow {
                index: 0,
                token: 31,
                name_key: b"PP".to_vec(),
                category_mask: u32::MAX,
                flags: crate::modifier_text::POSITIVE_IS_GOOD,
                precision: 2,
                ..Default::default()
            },
        ]));
    (fixture, folder)
}

fn scopes() -> EventScopes {
    let mut scope = EventScope::of(EventObject::IndustrialOrganisation(MIO));
    scope.root = Link::Scope(1);
    EventScopes::new(vec![scope, EventScope::of(EventObject::Country(1))])
}

#[test]
fn mio_missing_traits_keep_raw_token_and_show_modifiers_can_be_disabled() {
    game::enter();
    let (fixture, _folder) = fixture();
    for (body, expected) in [
        (
            "fixture_show_organisation_trait={trait=synthetic_convoy_production show_modifiers=no}",
            "Unlock Owner trait AAA\n",
        ),
        (
            "fixture_show_organisation_trait={trait=synthetic_convoy_production show_modifiers=no show_modifiers=YES}",
            "Unlock Owner trait AAA\n",
        ),
        (
            "fixture_finish_organisation_trait=unknown_trait",
            "Complete unknown_trait AAA\n",
        ),
        (
            "fixture_finish_organisation_trait=UNKNOWN_TRAIT",
            "Complete UNKNOWN_TRAIT AAA\n",
        ),
        (
            "fixture_finish_organisation_trait=initial_trait",
            "Complete initial_trait AAA\n",
        ),
    ] {
        let out = fixture.render(body, &mut scopes(), Content::default());
        assert_eq!(out.text, expected, "{body}");
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
    let out = fixture.render(
        "fixture_finish_organisation_trait=unknown_trait",
        &mut EventScopes::new(vec![EventScope::of(EventObject::Country(1))]),
        Content::default(),
    );
    assert_eq!(out.text, "Complete unknown_trait AAA\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn mio_catalogue_is_cached_once_per_stack() {
    game::enter();
    let (fixture, folder) = fixture();
    let content = fixture
        .catalogue
        .scripted_content()
        .unwrap()
        .effects()
        .unwrap();
    std::fs::write(
        folder
            .0
            .join("common/military_industrial_organization/organizations/test.txt"),
        "synthetic_mio={trait={token=synthetic_empty_trait name=WRONG}}",
    )
    .unwrap();
    assert!(std::ptr::eq(
        content,
        fixture
            .catalogue
            .scripted_content()
            .unwrap()
            .effects()
            .unwrap()
    ));
    let out = fixture.render(
        "fixture_finish_organisation_trait=synthetic_empty_trait",
        &mut scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Complete Unmodified trait AAA ()\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn mio_repeated_trait_and_valid_boolean_assignments_keep_the_last_value() {
    game::enter();
    let (fixture, _folder) = fixture();
    for body in [
        "fixture_show_organisation_trait={trait=unknown_trait trait=synthetic_convoy_production show_modifiers=no show_modifiers=yes}",
        "fixture_show_organisation_trait={trait=synthetic_convoy_production show_modifiers=no show_modifiers=\"yes\" show_modifiers=YES}",
    ] {
        let out = fixture.render(body, &mut scopes(), Content::default());
        assert_eq!(
            out.text,
            "Unlock Owner trait AAA (Soft+0.14, Supply--0.07, Power: \u{11}G+0.23\u{11}!\nRaw [Root.GetTag])\n"
        );
        assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
    }
}

#[test]
fn mio_production_bonus_uses_q15_rounding_and_quantized_zero_sign() {
    game::enter();
    let (fixture, _folder) = fixture();
    for (key, _, header) in EFFECTS {
        let out = fixture.render(
            &format!("{key}={{trait=production_gap}}"),
            &mut scopes(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!(
                "{} (Soft+0.14, Capacity: \u{11}G+30.00%\u{11}!, Cost: \u{11}G-0.00%\u{11}!, Power: \u{11}G+0.23\u{11}!\nRaw [Root.GetTag])\n",
                header.trim_end_matches('\n')
            ),
            "{key}"
        );
        assert!(out.refusals.is_empty(), "{key}: {:?}", out.refusals);
        let out = fixture.render(
            &format!("{key}={{trait=production_gap show_modifiers=no}}"),
            &mut scopes(),
            Content::default(),
        );
        assert_eq!(out.text, header);
        assert!(out.refusals.is_empty(), "{key}: {:?}", out.refusals);
    }
}

#[test]
fn mio_production_percent_keeps_sign_rounding_and_wrapping() {
    for (value, bad_positive, expected) in [
        (30_006, false, "\u{11}G+30.00%\u{11}!"),
        (29_999, false, "\u{11}G+30.00%\u{11}!"),
        (-30_006, false, "\u{11}R-30.00%\u{11}!"),
        (30_006, true, "\u{11}R+30.00%\u{11}!"),
        (-30_006, true, "\u{11}G-30.00%\u{11}!"),
        (-1, false, "\u{11}G+0.00%\u{11}!"),
        (1, true, "\u{11}G-0.00%\u{11}!"),
        (3_125, false, "\u{11}G+3.12%\u{11}!"),
        (9_375, false, "\u{11}G+9.38%\u{11}!"),
        (429_496_729_600_000, true, "\u{11}R+0.00%\u{11}!"),
    ] {
        assert_eq!(
            mio_production_percent(value, bad_positive),
            expected,
            "{value}/{bad_positive}"
        );
    }
}

#[test]
fn mio_policy_uses_the_same_production_bonus_formatter() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let mut database = content_reader_military::industrial_organisations::Database::default();
    database.policies.read_file(
        b"synthetic_policy={name=SYN_TRAIT production_bonus={SYN_TRAIT={production_capacity_factor=0.30006 production_cost_factor=-0.00001}}}",
        &content_reader_script::key_names::KeyNames::default(),
    ).unwrap();
    let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *fixture.frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    rows.push(LexicalTokenRow {
        token: 15,
        literal: b"synthetic_policy".to_vec(),
    });
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *fixture.frame.tables[TableId::CountryCounterInputs.index()]
    else {
        unreachable!()
    };
    rows[0].mio_policies = Recorded::Value(Some(vec![15]));
    let out = fixture.render(
        "fixture_offer_policy={policy=synthetic_policy}",
        &mut crate::effect_description::tests::fresh(),
        Content {
            industrial_organisations: Some(&database),
            ..Default::default()
        },
    );
    assert_eq!(
        out.text,
        "Policy Owner trait\n (\u{11}YBase trait\u{11}! Capacity: \u{11}G+30.00%\u{11}!, Cost: \u{11}G-0.00%\u{11}!)"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn mio_inherited_organisation_block_renders_copied_numeric_and_tooltip_entries() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *fixture.frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    tokens[0].literal = b"inherited_mio".to_vec();
    for (key, _, header) in EFFECTS {
        let out = fixture.render(
            &format!("{key}={{trait=synthetic_convoy_production}}"),
            &mut scopes(),
            Content::default(),
        );
        assert_eq!(
            out.text,
            format!(
                "{} (Soft+0.14, Supply--0.07, Power: \u{11}G+0.23\u{11}!\nRaw [Root.GetTag])\n",
                header.trim_end_matches('\n')
            ),
            "{key}"
        );
        assert!(out.refusals.is_empty(), "{key}: {:?}", out.refusals);
        let out = fixture.render(
            &format!("{key}={{trait=synthetic_convoy_production show_modifiers=no}}"),
            &mut scopes(),
            Content::default(),
        );
        assert_eq!(out.text, header);
        assert!(out.refusals.is_empty(), "{key}: {:?}", out.refusals);
    }
}

#[test]
fn mio_organisation_eligibility_or_formatter_error_keeps_bonus_fragments() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    *fixture.frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![]));
    for (trait_key, why) in [
        (
            "synthetic_convoy_production",
            "idea modifier political_power_gain has no recorded definition",
        ),
        (
            "numeric_zero",
            "trait modifier political_power_gain has no recorded definition",
        ),
    ] {
        for (key, label, header) in EFFECTS {
            let out = fixture.render(
                &format!("{key}={{trait={trait_key}}}"),
                &mut scopes(),
                Content::default(),
            );
            let bonuses = if trait_key == "numeric_zero" {
                "Soft+0.14"
            } else {
                "Soft+0.14, Supply--0.07"
            };
            assert_eq!(
                out.text,
                format!("{} ({bonuses})\n", header.trim_end_matches('\n')),
                "{key}/{trait_key}"
            );
            assert_eq!(
                out.refusals,
                [(format!("synthetic effect {label}: {why}"), 1)]
                    .into_iter()
                    .collect()
            );
        }
    }
}

#[test]
fn mio_effective_empty_gate_uses_the_recorded_feature_mask() {
    game::enter();
    let (mut fixture, _folder) = fixture();
    let TableView::Complete(TableRows::ModifierDefinitions(definitions)) =
        &mut *fixture.frame.tables[TableId::ModifierDefinitions.index()]
    else {
        unreachable!()
    };
    definitions[0].feature_selector = 4;
    *fixture.frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 0,
            ..Default::default()
        }]));
    let out = fixture.render(
        "fixture_show_organisation_trait={trait=synthetic_convoy_production}",
        &mut scopes(),
        Content::default(),
    );
    assert_eq!(
        out.text,
        "Unlock Owner trait AAA (Soft+0.14, Supply--0.07, Raw [Root.GetTag])\n"
    );
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let out = fixture.render(
        "fixture_show_organisation_trait={trait=numeric_zero}",
        &mut scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Unlock Owner trait AAA (Soft+0.14)\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    let TableView::Complete(TableRows::SimulationFeatures(features)) =
        &mut *fixture.frame.tables[TableId::SimulationFeatures.index()]
    else {
        unreachable!()
    };
    features[0].dlc_mask = 4;
    let out = fixture.render(
        "fixture_show_organisation_trait={trait=numeric_zero}",
        &mut scopes(),
        Content::default(),
    );
    assert_eq!(out.text, "Unlock Owner trait AAA (Soft+0.14,)\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}
