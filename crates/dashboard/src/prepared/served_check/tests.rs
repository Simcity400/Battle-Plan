use super::*;

/// A tiny prepared store: each country's value at each hour it holds, null
/// where the hour was read without one, in the order the store wrote them.
struct Tiny(Vec<(&'static str, i32, Option<f64>)>);

impl Tiny {
    fn serve(&self, product: &str, params: &Value) -> Reply {
        match product {
            "country-identities" => {
                let countries: serde_json::Map<String, Value> = self
                    .0
                    .iter()
                    .map(|(tag, _, _)| ((*tag).to_owned(), json!({})))
                    .collect();
                Reply::Answered(json!({ "data": { "countries": countries } }))
            }
            "toy-series" => {
                let (Some(first), Some(last), Some(tags)) = (
                    params["firstHour"].as_i64(),
                    params["lastHour"].as_i64(),
                    params["tags"].as_array(),
                ) else {
                    return Reply::Refused("Invalid toy request".into());
                };
                let series: Vec<Value> = tags
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|tag| {
                        let mut runs: Vec<(i32, Option<f64>)> = self
                            .0
                            .iter()
                            .filter(|(held, hour, _)| {
                                *held == tag && (first..=last).contains(&i64::from(*hour))
                            })
                            .map(|&(_, hour, value)| (hour, value))
                            .collect();
                        runs.sort_by_key(|run| run.0);
                        let runs: Vec<Value> = runs
                            .into_iter()
                            .map(|(hour, value)| json!([hour, value]))
                            .collect();
                        json!({ "tag": tag, "runs": runs })
                    })
                    .collect();
                let mut written: Vec<&str> = Vec::new();
                for &(tag, _, _) in &self.0 {
                    if !written.contains(&tag) {
                        written.push(tag);
                    }
                }
                Reply::Answered(json!({ "data": { "series": series, "written": written } }))
            }
            _ => Reply::Refused(format!("Unknown prepared query: {product}")),
        }
    }
}

/// Each country's whole history, its hours alone, and the order the store
/// wrote its countries in, which means nothing.
static TOY: Plan = Plan {
    queries: toy,
    unordered: &[&["written"]],
};

fn toy(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        let tags = ask.countries(branch.id)?;
        let mut spans = branch.spans();
        spans.extend(branch.hours().into_iter().map(|hour| (hour, hour)));
        for (first, last) in spans {
            ask.ask(
                "toy-series",
                json!({ "branchId": branch.name(), "tags": tags, "firstHour": first, "lastHour": last }),
            )?;
        }
    }
    Ok(())
}

fn branch() -> Branch {
    Branch {
        id: 1,
        first_hour: 0,
        last_hour: 3,
        windows: vec![(0, 1), (2, 3)],
        gaps: Vec::new(),
        segments: Vec::new(),
    }
}

fn facts() -> Vec<(&'static str, i32, Option<f64>)> {
    vec![
        ("ENG", 0, Some(2.0)),
        ("ENG", 1, None),
        ("ENG", 2, Some(2.5)),
        ("ENG", 3, Some(3.0)),
        ("GER", 0, Some(1.0)),
        ("GER", 1, Some(1.25)),
        ("GER", 2, Some(1.5)),
        ("GER", 3, Some(0.75)),
    ]
}

/// The reference's transcript, checked against the candidate.
fn compare(reference: &Tiny, candidate: &Tiny) -> Report {
    let mut transcript: Vec<u8> = Vec::new();
    let recorded = record(
        "toy",
        "1",
        &[Owner::Plan("toy-series", &TOY)],
        &[branch()],
        &mut |product: &str, params: &Value| reference.serve(product, params),
        &mut transcript,
    )
    .unwrap();
    assert!(
        recorded.iter().all(|owner| !owner.unchecked()),
        "{recorded:?}"
    );
    let mut lines = transcript.as_slice();
    let header = read_header(&mut lines).unwrap();
    assert_eq!(
        (header.family.as_str(), header.branches),
        ("toy", vec![branch()])
    );
    check(&mut lines, &mut |product: &str, params: &Value| {
        candidate.serve(product, params)
    })
    .unwrap()
}

fn first_difference(report: &Report) -> &Mismatch {
    report.products["toy-series"]
        .first
        .as_ref()
        .expect("the replies differ")
}

#[test]
fn a_store_written_in_another_order_passes() {
    let mut reordered = facts();
    reordered.reverse();
    let report = compare(&Tiny(facts()), &Tiny(reordered));
    assert_eq!(report.differing, 0, "{report:#?}");
    assert!(report.products["toy-series"].answered > 6);
    assert!(
        !report.products.contains_key("country-identities"),
        "discovery is not compared"
    );
}

#[test]
fn a_changed_value_fails_at_its_path() {
    let mut changed = facts();
    changed[6].2 = Some(1.375);
    let report = compare(&Tiny(facts()), &Tiny(changed));
    let first = first_difference(&report);
    assert_eq!(
        first.difference,
        Difference {
            path: "data.series[1].runs[2][1]".into(),
            reference: Side::Value(json!(1.5)),
            candidate: Side::Value(json!(1.375)),
        }
    );
    assert_eq!(
        (
            first.params["firstHour"].as_i64(),
            first.params["lastHour"].as_i64()
        ),
        (Some(0), Some(3))
    );
    assert!(
        !report.products.contains_key("country-identities"),
        "discovery is not compared"
    );
}

/// A plan that checks the identities themselves asks for them.
static IDENTITIES: Plan = Plan {
    queries: identities,
    unordered: &[],
};

fn identities(ask: &mut dyn Ask) -> io::Result<()> {
    for branch in ask.branches().to_vec() {
        ask.ask(
            "country-identities",
            json!({ "branchId": branch.id.to_string() }),
        )?;
    }
    Ok(())
}

#[test]
fn identities_a_plan_asks_for_are_recorded_and_compared() {
    let reference = Tiny(facts());
    let mut transcript: Vec<u8> = Vec::new();
    record(
        "identities",
        "1",
        &[Owner::Plan("country-identities", &IDENTITIES)],
        &[branch()],
        &mut |product: &str, params: &Value| reference.serve(product, params),
        &mut transcript,
    )
    .unwrap();
    let mut lines = transcript.as_slice();
    read_header(&mut lines).unwrap();
    let candidate = Tiny(facts());
    let report = check(&mut lines, &mut |product: &str, params: &Value| {
        candidate.serve(product, params)
    })
    .unwrap();
    assert_eq!(report.products["country-identities"].answered, 1);
    assert_eq!(report.differing, 0, "{report:#?}");
}

#[test]
fn a_value_missing_in_one_and_zero_in_the_other_fails() {
    let mut zero = facts();
    zero[1].2 = Some(0.0);
    let report = compare(&Tiny(facts()), &Tiny(zero));
    assert_eq!(
        first_difference(&report).difference,
        Difference {
            path: "data.series[0].runs[1][1]".into(),
            reference: Side::Value(Value::Null),
            candidate: Side::Value(json!(0.0)),
        }
    );
}

#[test]
fn a_missing_hour_fails_at_its_path() {
    let mut missing = facts();
    missing.remove(7);
    let report = compare(&Tiny(facts()), &Tiny(missing));
    assert_eq!(
        first_difference(&report).difference,
        Difference {
            path: "data.series[1].runs[3]".into(),
            reference: Side::Value(json!([3, 0.75])),
            candidate: Side::Absent,
        }
    );
    assert!(report.products["toy-series"].differing > 1);
}

#[test]
fn numbers_compare_by_their_exact_value_and_absent_apart_from_null() {
    let at = |reference: Value, candidate: Value| {
        difference(
            &Reply::Answered(json!({ "data": reference })),
            &Reply::Answered(json!({ "data": candidate })),
            &[],
        )
    };
    assert_eq!(at(json!(1), json!(1.0)), None);
    assert!(at(json!(0.1 + 0.2), json!(0.3)).is_some());
    assert!(at(json!(0), json!(-0.0)).is_some());
    assert!(at(json!(null), json!(0)).is_some());
    assert!(at(json!({}), json!({ "value": null })).is_some());
    assert!(at(json!([1, 2]), json!([2, 1])).is_some());
    assert_eq!(
        difference(
            &Reply::Refused("no".into()),
            &Reply::Refused("no".into()),
            &[]
        ),
        None
    );
    assert!(
        difference(
            &Reply::Refused("no".into()),
            &Reply::Answered(json!({})),
            &[]
        )
        .is_some()
    );
}

#[test]
fn a_plan_the_store_refuses_checks_nothing() {
    let store = Tiny(facts());
    let recorded = record(
        "toy",
        "1",
        &[Owner::Generic("unknown-product")],
        &[branch()],
        &mut |product: &str, params: &Value| store.serve(product, params),
        &mut Vec::<u8>::new(),
    )
    .unwrap();
    assert!(recorded[0].unchecked() && recorded[0].generic);
    assert_eq!(
        recorded[0].first_refusal.as_deref(),
        Some("unknown-product: Unknown prepared query: unknown-product")
    );
}

#[test]
fn the_families_on_columnar_segments_serve_by_plans_of_their_own() {
    for family in [
        "army",
        "fuel",
        "diplomacy_state",
        "equipment_losses",
        "occupation",
        "leaders",
        "event_feed",
        "stability",
        "supply",
        "division-log",
        "division-equipment",
        "faction_state",
        "naval",
        "politics_ledger",
        "stockpile",
    ] {
        let owners = owners(family).unwrap();
        assert!(!owners.is_empty(), "{family} serves nothing");
        for owner in owners {
            if let Owner::Generic(name) = owner {
                // The manifest is not a product of its own: its branch-wide request is all it takes.
                assert_eq!(
                    name, "ships-sunk-manifest",
                    "{family} serves {name} by the generic plan"
                );
            }
        }
    }
}
