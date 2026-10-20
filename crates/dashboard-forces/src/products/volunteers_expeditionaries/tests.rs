use super::*;
#[test]
fn military_aid_intervals_preserve_empty_unknown_and_segment_boundaries() {
    let proof = [Run::new(0, 2, 1, true), Run::new(4, 6, 1, true)];
    let values = [Run::new(1, 2, 2, false), Run::new(4, 4, 1, true)];
    let runs = project(&values, [&proof, &proof, &proof], 0, 6);
    assert_eq!(
        runs.iter()
            .map(|run| (
                run.first_hour,
                run.last_hour,
                run.divisions,
                run.starts_segment
            ))
            .collect::<Vec<_>>(),
        [
            (0, 0, Some(0.0), true),
            (1, 2, Some(2.0), false),
            (3, 3, None, false),
            (4, 4, Some(1.0), true),
            (5, 6, Some(0.0), false)
        ]
    );
}

#[test]
fn military_aid_refuses_air_and_oversized_replies_without_truncating() {
    assert!(
        serde_json::from_value::<VolunteersExpeditionariesParams>(
            serde_json::json!({"branchId":"1","kind":"air","tags":null})
        )
        .is_err()
    );
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let actor = super::super::army_size::aid::Actor {
        slot: 1,
        id: 1,
        tag: "x".repeat(dashboard_base::products::MAX_PRODUCT_REPLY_BYTES),
    };
    let catalogue = Catalogue {
        actors: vec![actor],
        links: vec![super::super::army_size::aid::Link {
            sender: 0,
            host: 0,
            channels: 1,
        }],
        ..Default::default()
    };
    let prefix = prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: 0,
            last_hour: 1,
            complete: true,
        },
        pending: Vec::new(),
    };
    let params = VolunteersExpeditionariesParams {
        branch_id: "1".into(),
        kind: MilitaryAidKind::Land,
        tags: None,
    };
    let error = read(
        &fixture.store,
        &fixture.key,
        1,
        &params,
        catalogue,
        prefix,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.to_string().contains("reply budget"));
}
