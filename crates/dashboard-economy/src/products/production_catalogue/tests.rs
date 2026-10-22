use super::*;
use crate::products::test_support::*;
use replay_data::rows::*;

fn base(hour: i32) -> DecodedFrame {
    let mut at = frame(hour);
    countries(&mut at, &[(1, 10, "LIC"), (2, 20, "GIV"), (3, 30, "OTH")]);
    at
}

fn licence(owner: u32, ordinal: u32, giver: u32, lent: i32, equipment: Vec<u64>) -> ProductionLicenseRecordRow {
    ProductionLicenseRecordRow {
        owner,
        ordinal,
        giver,
        lended_cic: lent,
        required_cic: 2,
        start_date: 900,
        parent: 0,
        operand_raw: 77,
        equipment,
    }
}

fn design(id: u64, creator: u32, obsolete: u8) -> VariantDefinitionRow {
    VariantDefinitionRow {
        id,
        creator_link: creator,
        obsolete,
        ..Default::default()
    }
}

fn ask(state: &mut Prepared, view: CatalogueView, tags: &[&str], through: i32) -> ProductionCatalogueReply {
    let fixture = fixture();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    read(&fixture, view, tags, through)
}

fn read(
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    view: CatalogueView,
    tags: &[&str],
    through: i32,
) -> ProductionCatalogueReply {
    let flag = AtomicBool::new(false);
    query(
        &fixture.store,
        &fixture.key,
        &ProductionCatalogueParams {
            branch_id: "1".into(),
            view,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        },
        Some(through),
        &Naming {
            texts: None,
            units: None,
            icon: &|_| None,
        },
        Vec::new(),
        &flag,
    )
    .unwrap()
}

#[test]
fn licences_keep_licensor_known_zero_and_every_variant_in_order() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(
        &mut at,
        TableRows::ProductionLicenseRecords(vec![
            licence(1, 0, 2, 0, vec![0xc1, 0xc2]),
            licence(3, 0, 3, 1, vec![0xc3]),
        ]),
    );
    state.step(&at);
    // The record stops being read: its end is bounded, not dated further.
    let mut at = base(6);
    set(&mut at, TableRows::ProductionLicenseRecords(vec![]));
    state.step(&at);
    let reply = ask(&mut state, CatalogueView::Licences, &["GIV"], 6);
    assert_eq!(reply.licences.len(), 1);
    let licence = &reply.licences[0];
    assert_eq!(licence.licensee, "10");
    assert_eq!(licence.licensor, vec![EconomyRun(5, 5, "20".into())]);
    // Zero factories lent is a real licence.
    assert_eq!(licence.lent[0].2, "0");
    assert_eq!(licence.variants[0][0].2, "c1");
    assert_eq!(licence.variants[1][0].2, "c2");
    assert!(reply.holders.contains_key("20"));
}

#[test]
fn organisations_keep_unknown_state_apart_and_a_failed_detail_read_spares_licences() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(
        &mut at,
        TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow {
            organisation: u64::MAX,
            country: 1,
            ..Default::default()
        }]),
    );
    set(
        &mut at,
        TableRows::IndustrialOrganisationDetails(vec![IndustrialOrganisationDetailsRow {
            organisation: u64::MAX,
            country: 1,
            state: IndustrialOrganisationState {
                size: 3,
                funds_raw: -5,
                // The game's "no policy selected".
                policy: 0x4c17,
                ..Default::default()
            },
            unlocked: vec![1, 2],
            ..Default::default()
        }]),
    );
    set(&mut at, TableRows::ProductionLicenseRecords(vec![licence(1, 0, 2, 1, vec![1])]));
    state.step(&at);
    let mut at = base(6);
    set(
        &mut at,
        TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow {
            organisation: u64::MAX,
            country: 1,
            ..Default::default()
        }]),
    );
    unavailable(&mut at, TableId::IndustrialOrganisationDetails);
    set(&mut at, TableRows::ProductionLicenseRecords(vec![licence(1, 0, 2, 1, vec![1])]));
    state.step(&at);
    let fixture = fixture();
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    let reply = read(&fixture, CatalogueView::Organisations, &["LIC"], 6);
    let mio = &reply.organisations[0];
    assert_eq!(mio.id, u64::MAX.to_string());
    assert_eq!(mio.holder, vec![EconomyRun(5, 6, "10".into())]);
    // The unread hour has no size, never zero.
    assert_eq!(mio.size, vec![EconomyRun(5, 5, "3".into())]);
    assert_eq!(mio.funds[0].2, "-5");
    assert_eq!(mio.unlocked[0].2, "2");
    assert!(mio.policy.is_empty());
    assert!(reply.warnings.iter().any(|line| line.contains("organisation state unavailable")));
    let licences = read(&fixture, CatalogueView::Licences, &[], 6);
    assert_eq!(licences.licences[0].lent, vec![EconomyRun(5, 6, "1".into())]);
}

#[test]
fn designs_date_obsolescence_to_the_read_that_shows_it_and_mark_the_first_read() {
    let mut state = Prepared::default();
    for (hour, rows) in [
        (5, vec![design(1, 1, 0), design(2, 1, 1)]),
        (6, vec![design(1, 1, 0), design(2, 1, 1)]),
        (7, vec![design(1, 1, 1), design(2, 1, 1), design(3, 1, 0), design(4, 3, 0)]),
    ] {
        let mut at = base(hour);
        set(&mut at, TableRows::VariantDefinitions(rows));
        state.step(&at);
    }
    let reply = ask(&mut state, CatalogueView::Designs, &["LIC"], 7);
    assert_eq!(reply.designs_from, Some(5));
    let ids: Vec<_> = reply.designs.iter().map(|design| design.id.as_str()).collect();
    assert_eq!(ids, vec!["1", "2", "3"]);
    assert_eq!(
        reply.designs[0].obsolete,
        vec![EconomyRun(5, 6, "0".into()), EconomyRun(7, 7, "1".into())]
    );
    assert_eq!(reply.designs[2].creator, vec![EconomyRun(7, 7, "10".into())]);
}

#[test]
fn five_thousand_countries_keep_their_designs_sparse() {
    let mut state = Prepared::default();
    let mut at = frame(5);
    many_countries(&mut at, 5_000);
    set(
        &mut at,
        TableRows::VariantDefinitions((0..1_000).map(|n| design(u64::from(n) + 1, n * 5 + 1, 0)).collect()),
    );
    state.step(&at);
    assert_eq!(ask(&mut state, CatalogueView::Designs, &["T0005"], 5).designs.len(), 1);
}

#[test]
fn a_refused_roster_is_unknown_not_empty_in_every_domain() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(&mut at, TableRows::ProductionLicenseRecords(vec![licence(1, 0, 2, 1, vec![1])]));
    set(&mut at, TableRows::VariantDefinitions(vec![design(1, 1, 0)]));
    state.step(&at);
    let mut at = base(6);
    unavailable(&mut at, TableId::ProductionLicenseRecords);
    unavailable(&mut at, TableId::IndustrialOrganisations);
    unavailable(&mut at, TableId::VariantDefinitions);
    state.step(&at);
    let fixture = fixture();
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    for view in [CatalogueView::Licences, CatalogueView::Organisations, CatalogueView::Designs] {
        let reply = read(&fixture, view, &[], 6);
        // Complete at 5 (an empty organisation roster is known empty), unknown at 6.
        assert_eq!(reply.known, vec![EconomyRun(5, 5, "1".into())], "{view:?}");
    }
}

#[test]
fn a_selected_policy_without_a_spelling_is_unresolved_not_none() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(
        &mut at,
        TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow { organisation: 7, country: 1, ..Default::default() }]),
    );
    set(
        &mut at,
        TableRows::IndustrialOrganisationDetails(vec![IndustrialOrganisationDetailsRow {
            organisation: 7,
            country: 1,
            // Selected (not the no-policy sentinel), but the tokens table does not spell it.
            state: IndustrialOrganisationState { size: 1, policy: 99, ..Default::default() },
            ..Default::default()
        }]),
    );
    state.step(&at);
    let reply = ask(&mut state, CatalogueView::Organisations, &["LIC"], 5);
    let mio = &reply.organisations[0];
    assert_eq!(mio.id, "7");
    assert!(mio.policy.is_empty());
    assert_eq!(mio.policy_unresolved, vec![EconomyRun(5, 5, "1".into())]);
}

#[test]
fn a_failed_token_dictionary_refuses_only_organisation_names() {
    let mut state = Prepared::default();
    let roster = || TableRows::IndustrialOrganisations(vec![IndustrialOrganisationRow { organisation: 7, country: 1, ..Default::default() }]);
    let details = || TableRows::IndustrialOrganisationDetails(vec![IndustrialOrganisationDetailsRow {
        organisation: 7,
        country: 1,
        state: IndustrialOrganisationState { size: 4, policy: 0x4c17, ..Default::default() },
        ..Default::default()
    }]);
    for (hour, readable) in [(5, true), (6, false), (7, true)] {
        let mut at = base(hour);
        set(&mut at, roster());
        set(&mut at, details());
        if !readable {
            unavailable(&mut at, TableId::LexicalTokens);
        }
        state.step(&at);
    }
    let reply = ask(&mut state, CatalogueView::Organisations, &["LIC"], 7);
    // The roster stays known and its numbers stay read through the failure.
    assert_eq!(reply.known, vec![EconomyRun(5, 7, "1".into())]);
    let mio = &reply.organisations[0];
    assert_eq!(mio.size, vec![EconomyRun(5, 7, "4".into())]);
    // The names are unknown from the failed read and named again after it.
    assert_eq!(mio.names.iter().map(|name| name.first_hour).collect::<Vec<_>>(), vec![5, 6, 7]);
    assert_eq!(mio.names[1].name, None);
    assert!(reply.warnings.iter().any(|line| line.contains("organisation names unavailable")));
}
