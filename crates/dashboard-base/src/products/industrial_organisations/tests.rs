use super::*;
use engine_executable::modifier_keys::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST as KEY;
use replay_data::rows::{TableId, TableRows};
use replay_frame::frame::{Metrics, TableView};

#[test]
fn attach_quote_sums_before_scaling_and_rounds_half_away_from_zero() {
    assert_eq!(attach_quote(25, -4_000, -6_000), 23);
    assert_eq!(attach_quote(25, -52_000, -50_000), -1);
    assert_eq!(attach_quote(25, 0, 0), 25);
    assert_eq!(attach_quote(0, 70_000, 30_000), 0);
    assert_eq!(attach_quote(3, 16_000, 17_000), 4);
    assert_eq!(attach_quote(-7, 0, 50_000), -11);
}

#[test]
fn attach_quote_keeps_every_native_narrowing_under_overflow() {
    let wrap64 = |value: i128| value as i64;
    let reference = |base: i32, local: i64, country: i64| {
        let sum = wrap64(i128::from(ONE) + i128::from(local) + i128::from(country));
        let factor = wrap64(i128::from(sum) * i128::from(ONE)) / ONE;
        let raw =
            wrap64(i128::from(wrap64(i128::from(base) * i128::from(factor))) * i128::from(ONE))
                / ONE;
        let half = if raw < 0 { -50_000 } else { 50_000 };
        (wrap64(i128::from(raw) + half) / ONE) as i32
    };
    for (base, local, country) in [
        (25, 1 << 62, 0),
        (i32::MAX, 200_000, 0),
        (i32::MIN, i64::MAX, i64::MAX),
        (-1, i64::MIN, -ONE),
        (40, 92_233_720_368_547, 0),
    ] {
        assert_eq!(
            attach_quote(base, local, country),
            reference(base, local, country),
            "{base} {local} {country}"
        );
    }
    let exact = (25 * (i128::from(ONE) + (1 << 62)) + 50_000) / i128::from(ONE);
    assert_ne!(i128::from(attach_quote(25, 1 << 62, 0)), exact);
}

#[test]
fn affordability_compares_the_exact_pool_with_the_rescaled_quote() {
    assert!(affordable(2_300_000, 23));
    assert!(!affordable(2_299_999, 23));
    assert!(affordable(-100_000, -1));
    assert!(!affordable(-100_001, -1));
    assert!(shows_attach_cost(1, false));
    assert!(!shows_attach_cost(1, true));
    assert!(!shows_attach_cost(0, false));
    assert!(!shows_attach_cost(-3, false));
}

const ORGANISATION: u64 = (7 << 32) | 79;
const HOLDER: u32 = 4_999;
const OWNER_ALIAS: u32 = 5_000;
const OWNER: u32 = 6_000;

fn frame(flags: u32, owner_modifier: Option<i64>) -> DecodedFrame {
    let modifier = engine_executable::modifier_keys::fixed_name(KEY).unwrap();
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "missing fixture input".into(),
            }
            .into()
        }),
    };
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (100, "synthetic_org"),
            (200, "unlocked"),
            (201, "unknown_trait"),
            (300, "attached_policy"),
            (301, "candidate_policy"),
            (400, modifier),
            (500, "synthetic_flag"),
        ]
        .into_iter()
        .map(|(token, literal)| rows::LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        })
        .collect(),
    ));
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            rows::ModifierDefinitionRow {
                index: KEY,
                token: 400,
                flags,
                ..Default::default()
            },
        ]));
    let policy = |token, cost| rows::IndustrialOrganisationPolicy {
        token,
        cost,
        ..Default::default()
    };
    *frame.tables[TableId::IndustrialOrganisationPolicies.index()] =
        TableView::Complete(TableRows::IndustrialOrganisationPolicies(vec![
            rows::IndustrialOrganisationPoliciesRow {
                country: HOLDER,
                policies: Vec::new(),
                cost_modifier_raw: Some(-ONE).into(),
            },
            rows::IndustrialOrganisationPoliciesRow {
                country: OWNER,
                policies: vec![policy(300, 50), policy(301, 25)],
                cost_modifier_raw: owner_modifier.into(),
            },
            rows::IndustrialOrganisationPoliciesRow {
                country: OWNER_ALIAS,
                policies: Vec::new(),
                cost_modifier_raw: Some(-ONE).into(),
            },
        ]));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        [(HOLDER, 3, "HLD"), (OWNER, 7, "OWN")]
            .into_iter()
            .map(|(id, canonical_slot, tag)| rows::CountryRow {
                id,
                canonical_slot,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(HOLDER, 3), (OWNER_ALIAS, 7), (OWNER, 7)]
            .into_iter()
            .map(|(id, canonical_slot)| rows::CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    *frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![organisation()]));
    frame
}

fn organisation() -> rows::IndustrialOrganisationRow {
    rows::IndustrialOrganisationRow {
        organisation: ORGANISATION,
        country: HOLDER,
        owner: OWNER_ALIAS,
        token: 100,
        ..Default::default()
    }
}

fn details(policy: u32) -> rows::IndustrialOrganisationDetailsRow {
    rows::IndustrialOrganisationDetailsRow {
        organisation: ORGANISATION,
        country: HOLDER,
        unlocked: vec![200, 201],
        state: rows::IndustrialOrganisationState {
            policy,
            ..Default::default()
        },
        ..Default::default()
    }
}

struct Content {
    root: std::path::PathBuf,
    traits: content_reader_military::mio_traits::Catalog,
    policies: content_reader_military::industrial_organisations::policies::Catalog,
}
impl Content {
    fn read(available: &str) -> Self {
        Self::with_visible("always=yes", available)
    }
    fn with_visible(visible: &str, available: &str) -> Self {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("modifier name tests require BATTLEPLAN_GAME_DIR with the supported installation");
        let modifier = engine_executable::modifier_keys::fixed_name(KEY).unwrap();
        let root = std::env::temp_dir().join(format!("mio_policy_quote_{}", uuid::Uuid::new_v4()));
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "common/military_industrial_organization/organizations/synthetic.txt",
            &format!(
                "synthetic_org={{visible={{{visible}}} available={{always={available}}} initial_trait={{token=initial organization_modifier={{{modifier}=0.05}}}} trait={{token=unlocked organization_modifier={{{modifier}=-0.3}}}}}}"
            ),
        );
        write(
            "common/military_industrial_organization/policies/synthetic.txt",
            &format!(
                "attached_policy={{organization_modifier={{{modifier}=-0.1}}}} candidate_policy={{organization_modifier={{{modifier}=-0.5}}}}"
            ),
        );
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let names = content_reader_key_names::key_name_products::KeyNameProducts::read(&layers).unwrap();
        Self {
            traits: content_reader_military::mio_traits::read(&layers).unwrap(),
            policies: content_reader_military::industrial_organisations::read(&layers, &names)
                .unwrap()
                .policies,
            root,
        }
    }
}
impl Drop for Content {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn quote(
    content: &Content,
    frame: &DecodedFrame,
    policy: u32,
    base: i32,
) -> Result<(i64, i32), String> {
    use dashboard_text::triggers::{ContentFacts, World};
    let dlcs = Default::default();
    let triggers = Default::default();
    let aliases = Default::default();
    let world = World::read(
        frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    );
    let (organisation, details) = (organisation(), details(policy));
    let local = local_modifier(
        frame,
        &organisation,
        &details,
        KEY,
        &content.traits,
        &content.policies,
        Ok(&world),
    )?;
    let cost = policy_cost(
        frame,
        &organisation,
        &details,
        &content.traits,
        &content.policies,
        Ok(&world),
    )?;
    assert_eq!(cost.local, local);
    Ok((local, cost.quote(base)))
}

#[test]
fn policy_quote_joins_gated_traits_initial_trait_attached_policy_and_owner_modifier() {
    let content = Content::read("yes");
    let frame = frame(0, Some(-20_000));
    assert_eq!(quote(&content, &frame, 300, 25), Ok((-35_000, 11)));
    assert_eq!(quote(&content, &frame, 0x4c17, 25), Ok((-25_000, 14)));
    let clamped = self::frame(
        engine_native::modifier_definition::CLAMP_MAXIMUM,
        Some(-20_000),
    );
    assert_eq!(quote(&content, &clamped, 300, 25), Ok((5_000, 21)));
    let unknown = self::frame(0, None);
    let refusal = quote(&content, &unknown, 300, 25).unwrap_err();
    assert!(
        refusal.contains("policy cost modifier is not recorded"),
        "{refusal}"
    );
}

#[test]
fn completed_traits_need_the_organisation_available_and_the_initial_trait_does_not() {
    let content = Content::read("no");
    let frame = frame(0, Some(0));
    assert_eq!(quote(&content, &frame, 300, 20), Ok((-5_000, 19)));
}

#[test]
fn organisation_conditions_read_the_owner_through_from() {
    let content = Content::with_visible("FROM={always=yes}", "yes");
    let frame = frame(0, Some(-20_000));
    assert_eq!(quote(&content, &frame, 300, 25), Ok((-35_000, 11)));
    let content = Content::with_visible("ROOT={PREV={always=yes}}", "yes");
    assert_eq!(quote(&content, &frame, 300, 25), Ok((-35_000, 11)));
    let content = Content::with_visible("FROM={always=yes}", "yes");
    let mut unlinked = frame;
    *unlinked.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![rows::CountryLinkRow {
            id: HOLDER,
            canonical_slot: 3,
        }]));
    assert!(quote(&content, &unlinked, 300, 25).is_err());
}

#[test]
fn a_false_organisation_condition_decides_whatever_the_other() {
    let unknown = || Err::<bool, String>("unknown".into());
    assert_eq!(both(unknown(), || Ok(false)), Ok(false));
    assert_eq!(both(Ok(false), || panic!("not evaluated")), Ok(false));
    assert_eq!(both(Ok(true), || Ok(false)), Ok(false));
    assert_eq!(both(Ok(true), || Ok(true)), Ok(true));
    assert!(both(unknown(), || Ok(true)).is_err());
    assert!(both(Ok(true), unknown).is_err());
}

#[test]
fn an_undecided_visible_condition_with_a_false_available_one_excludes_the_traits() {
    let content = Content::with_visible("has_mio_flag=synthetic_flag", "yes");
    assert!(quote(&content, &frame(0, Some(0)), 300, 20).is_err());
    let content = Content::with_visible("has_mio_flag=synthetic_flag", "no");
    let frame = frame(0, Some(0));
    assert_eq!(quote(&content, &frame, 300, 20), Ok((-5_000, 19)));
}
