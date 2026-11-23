use super::*;

#[test]
fn bare_empty_variable_keys_keep_values_and_scope_presence() {
    let saved = SaveDocument::read(
        br#"HOI4txt
        date=1936.1.1.1 game_unique_id=c version=b
        variables={=0 named=1}
        countries={X={variables={random={1 2} =-0.00001 ""=2}} Y={variables={}} Z={}}
        states={1={variables={=90071992547.40993}} 2={variables={}} 3={}}
        character_manager={historical={
            character={id={id=7 type=73} token=x variables={=0.00002}}
            character={id={id=8 type=73} token=y variables={}}
            character={id={id=9 type=73} token=z}
        }}
        raid={variables={=-92233720368547.75808}}"#,
    )
    .unwrap();
    let intel = super::super::intel::parse(saved.items()).unwrap();
    let global = intel.global_variables.as_ref().unwrap();
    assert_eq!(global.entries[0].name, b"");
    assert_eq!(global.entries[0].value.raw, 0);
    assert_eq!(global.entries[1].name, b"named");
    let country = intel.countries[b"X".as_slice()].variables.as_ref().unwrap();
    assert_eq!(country.seed, Some(2));
    assert_eq!(country.entries.len(), 2);
    assert!(country.entries.iter().all(|entry| entry.name.is_empty()));
    assert_eq!(country.entries[0].value.raw, -1);
    assert_eq!(country.entries[1].value.raw, 200_000);
    assert_eq!(
        intel.countries[b"Y".as_slice()].variables,
        Some(Default::default())
    );
    assert!(intel.countries[b"Z".as_slice()].variables.is_none());
    let states = intel.state_variables.as_ref().unwrap();
    assert_eq!(
        states[&1].as_ref().unwrap().entries[0].value.raw,
        9_007_199_254_740_993
    );
    assert_eq!(states[&2], Some(Default::default()));
    assert_eq!(states[&3], None);
    let characters = super::super::characters::parse(saved.items())
        .unwrap()
        .manager
        .unwrap();
    let character = characters.characters[&((7 << 32) | 73)]
        .variables
        .as_ref()
        .unwrap();
    assert!(character.entries[0].name.is_empty());
    assert_eq!(character.entries[0].value.raw, 2);
    assert_eq!(
        characters.characters[&((8 << 32) | 73)].variables,
        Some(Default::default())
    );
    assert!(characters.characters[&((9 << 32) | 73)].variables.is_none());
    let raid = required_block(saved.items(), b"raid").unwrap();
    let variables = required_block(raid, b"variables").unwrap();
    assert_eq!(variables[0].key.as_deref(), Some(b"".as_slice()));
    let parsed = super::super::intel::parse_variables(variables).unwrap();
    assert_eq!(parsed.entries[0].value.raw, i64::MIN);
}

#[test]
fn empty_variable_keys_do_not_relax_other_save_or_content_syntax() {
    for body in [
        "=0",
        "other={=0}",
        "variables={=}",
        "\"variables\"={=0}",
        "variables={=0} other={=0}",
        "variables>{=0}",
        "variables={=word}",
        "variables={=\"0\"}",
        "variables={={}}",
        "variables={==0}",
        "variables={random={=0}}",
        "variables={=0",
        "variables={=+0}",
        "variables={=0.000001}",
        "variables={=92233720368547.75808}",
    ] {
        let input = format!("HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b {body}");
        assert!(SaveDocument::read(input.as_bytes()).is_err(), "{body}");
    }
    assert!(content_reader_parser::parser::parse(b"variables={=0}").is_err());
    assert!(content_reader_parser::parser::parse_unclosed(b"variables={=0}").is_err());
    let saved =
        SaveDocument::read(b"HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b variables={0}")
            .unwrap();
    assert!(super::super::intel::parse(saved.items()).is_err());
}

fn document(countries: &str, extra: &str) -> SaveDocument {
    SaveDocument::read(
        format!(
            "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 \
         debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 \
         theater_group_index=6 military_deployment_line_index=7 \
         military_deployment_conveyor_index=8 equipment_variant_index=9 \
         country_leader_index=10 navy_id=11 land_combat_id=12 \
         multiplayer_random_count=-13 countries={{{countries}}} states={{}} provinces={{}} {extra}"
        )
        .as_bytes(),
    )
    .unwrap()
}

fn reserved(physical_slot: u32) -> CountryEmissionIdentity {
    CountryEmissionIdentity {
        physical_slot,
        own_id: 0,
        actual: ActualCountry::Reserved,
        tag: b"RES".to_vec(),
    }
}

#[test]
fn witnessed_repeats_keep_all_emissions_and_all_typed_country_packs() {
    let body = "strategic_navy={naval_base={9={} -4={} 9={}}} intel={} \
                division_template_id={id=7 type=56} command_power=0.0 unknown={2 1 2}";
    let repeated = document(&format!("RES={{{body}}} RES={{{body}}}"), "");
    assert!(repeated.parse().is_err());
    let identities = [reserved(1), reserved(2)];
    let representatives = repeated.representative_country_items(&identities).unwrap();
    assert_eq!(representatives.len(), 1);
    assert!(std::ptr::eq(
        representatives[0],
        &repeated.original_country_items().unwrap()[0],
    ));
    let mut projected = repeated.project(&identities).unwrap().parse().unwrap();
    assert_eq!(projected.countries.len(), 1);
    assert_eq!(projected.division_template_owners.len(), 1);
    assert_eq!(projected.economy.countries.len(), 1);
    assert_eq!(projected.governance.countries.len(), 1);
    assert_eq!(projected.intel.countries.len(), 1);
    assert_eq!(projected.projects.countries.len(), 1);
    assert_eq!(projected.characters.abilities.len(), 1);
    assert_eq!(projected.counter_inputs.prides.len(), 1);
    assert_eq!(projected.country_rosters.owned_goals.len(), 1);
    assert_eq!(projected.production.countries.as_ref().unwrap().len(), 1);
    assert_eq!(
        projected
            .naval_history
            .theatre_countries
            .as_ref()
            .unwrap()
            .len(),
        1
    );
    let emissions = projected.country_emissions.as_ref().unwrap();
    assert_eq!(emissions.len(), 2);
    assert!(emissions.iter().all(|entry| entry.intel_present));
    assert_eq!(emissions[1].identity.physical_slot, 2);
    assert_eq!(projected.country_naval_launch.len(), 2);
    assert_eq!(projected.country_naval_launch[1].ordinal, 1);
    assert_eq!(
        projected.country_naval_launch[0].naval_bases,
        Some(
            vec![9, -4, 9]
                .into_iter()
                .map(crate::naval_launch::NavalBaseOccurrence::from)
                .collect::<Vec<_>>()
        )
    );

    let unique = document(&format!("RES={{{body}}}"), "").parse().unwrap();
    projected.country_emissions = None;
    projected.country_naval_launch = unique.country_naval_launch.clone();
    assert_eq!(projected, unique);
}

#[test]
fn entire_outer_item_and_unrecognised_order_are_checked_before_projection() {
    for second in [
        "RES>{unknown={2 1 2}}",
        "\"RES\"={unknown={2 1 2}}",
        "RES={unknown={1 2 2}}",
        "RES={unknown={2 1}}",
        "RES={unknown={2 1 2} extra=0}",
    ] {
        let saved = document(&format!("RES={{unknown={{2 1 2}}}} {second}"), "");
        let identities = [reserved(1), reserved(2)];
        let error = saved.project(&identities).unwrap_err().to_string();
        assert!(saved.representative_country_items(&identities).is_err());
        assert!(error.contains("emissions 0 and 1"), "{error}");
        assert!(error.contains("differ at"), "{error}");
    }
    let saved = document("RES={command_power=0.0} RES={command_power=0}", "");
    assert!(saved.project(&[reserved(1), reserved(2)]).is_err());
}

#[test]
fn count_tag_order_actual_identity_and_local_corruption_remain_strict() {
    let saved = document("RES={} RES={}", "");
    assert!(saved.project(&[reserved(1)]).is_err());
    assert!(saved.project(&[reserved(2), reserved(1)]).is_err());
    let mut wrong_tag = reserved(2);
    wrong_tag.tag = b"OTHER".to_vec();
    assert!(saved.project(&[reserved(1), wrong_tag]).is_err());
    let mut distinct = reserved(2);
    distinct.own_id = 7;
    distinct.actual = ActualCountry::Canonical(7);
    assert!(saved.project(&[reserved(1), distinct]).is_err());

    for body in [
        "stability=0 stability=0",
        "intel={intel={}}",
        "division_template_id={id=7 type=56} division_template_id={id=7 type=56}",
    ] {
        let saved = document(&format!("RES={{{body}}} RES={{{body}}}"), "");
        let identities = [reserved(1), reserved(2)];
        assert!(
            saved.project(&identities).unwrap().parse().is_err(),
            "{body}"
        );
    }
}

#[test]
fn portrait_repeats_are_projected_but_global_duplicates_are_not() {
    let body = "navy_leader={id={id=7 type=73} portrait_path=\"role.dds\"}";
    let saved = document(&format!("RES={{{body}}} RES={{{body}}}"), "");
    let identities = [reserved(1), reserved(2)];
    let parsed = saved.project(&identities).unwrap().parse().unwrap();
    assert_eq!(parsed.characters.portraits.len(), 1);
    let saved = document(&format!("RES={{{body}}} RES={{{body}}}"), body);
    assert!(
        saved
            .project(&identities)
            .unwrap()
            .parse()
            .unwrap_err()
            .to_string()
            .contains("duplicate serialized leader portrait identity")
    );
}

#[test]
fn unrelated_global_failures_and_intel_object_presence_are_preserved() {
    let saved = document("RES={} RES={}", "states={}");
    assert!(
        saved
            .project(&[reserved(1), reserved(2)])
            .unwrap()
            .parse()
            .is_err()
    );
    let saved = document("RES={}", "");
    assert!(
        !saved
            .project(&[reserved(1)])
            .unwrap()
            .parse()
            .unwrap()
            .country_emissions
            .unwrap()[0]
            .intel_present
    );
    let saved = document("RES={intel={}}", "");
    let parsed = saved.project(&[reserved(1)]).unwrap().parse().unwrap();
    assert!(parsed.country_emissions.unwrap()[0].intel_present);
    assert!(parsed.country_intel.is_empty());
}
