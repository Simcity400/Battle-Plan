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
fn equipment_loss_plan_uses_witnessed_holders_and_reached_hours_with_1500_countries() {
    for last in [0, 999] {
        let mut branch = branch();
        branch.last_hour = last;
        branch.windows = vec![(0, last)];
        let mut countries = serde_json::Map::new();
        for id in 0..1500u32 {
            let tag: String = [id / 676, id / 26 % 26, id % 26]
                .into_iter()
                .map(|letter| char::from(b'A' + letter as u8))
                .collect();
            let first = if id == 1499 { last } else { 0 };
            countries.insert(
                tag,
                json!({ "identities": [
                { "slot": id, "countryId": id + 10, "firstHour": last, "lastHour": last },
                { "slot": id, "countryId": id + 10, "firstHour": first, "lastHour": last },
                { "slot": id, "countryId": id + 1510, "firstHour": last, "lastHour": last },
            ] }),
            );
        }
        let mut reached = HashSet::new();
        let mut navigated = HashSet::new();
        let mut transcript = Vec::new();
        let recorded = record(
            "equipment_losses",
            "1",
            &[Owner::Plan("equipment-losses", &crate::products::equipment_losses::served::PLAN)],
            &[branch],
            &mut |product, params| {
                if product == "country-identities" {
                    return Reply::Answered(json!({ "data": { "countries": countries } }));
                }
                let selection = if product == "equipment-losses" {
                    &params["selection"]
                } else {
                    &params["family"]["selection"]
                };
                let tag = selection["tag"].as_str().unwrap();
                let country = &selection["country"];
                let identities = countries[tag]["identities"].as_array().unwrap();
                let origin = identities.iter()
                    .filter(|identity| identity["slot"] == country["slot"] && identity["countryId"] == country["countryId"])
                    .map(|identity| identity["firstHour"].as_i64().unwrap())
                    .min().unwrap();
                assert_eq!(country["originHour"], origin);
                assert_eq!(selection["branchId"], "1");
                match product {
                    "equipment-losses" => {
                        reached.insert((tag.to_owned(), country["countryId"].as_u64().unwrap()));
                        let reads: Vec<_> = params["reads"].as_array().unwrap().iter()
                            .map(|hour| hour.as_i64().unwrap()).collect();
                        assert!(reads.windows(2).all(|pair| pair[0] < pair[1]));
                        for field in ["headHour", "detailHour"] {
                            assert!(reads.contains(&params[field].as_i64().unwrap()));
                        }
                        assert!((0..=last).contains(&(params["from"].as_i64().unwrap() as i32)));
                        assert!((0..=last).contains(&(params["to"].as_i64().unwrap() as i32)));
                        Reply::Answered(json!({ "data": {
                            "causes": [{ "source": { "kind": "sunkShips" }, "domain": "naval",
                                "name": { "key": null, "template": null, "text": null }, "group": null,
                                "loss": true, "unknown": null, "unknownSpans": [],
                                "leaves": [
                                    { "design": 0, "points": [], "lowerBoundFrom": null },
                                    { "design": null, "points": [], "lowerBoundFrom": null },
                                ] }],
                            "designs": [{ "variant": 99 }],
                        } }))
                    }
                    "series-next-change" => {
                        navigated.insert(params["key"].as_str().unwrap().to_owned());
                        let leaves = params["leaves"].as_array().unwrap();
                        assert_eq!(leaves[0], json!([{ "kind": "sunkShips" }, 99]));
                        if leaves.len() > 1 {
                            assert_eq!(leaves[1], json!([{ "kind": "sunkShips" }, null]));
                        }
                        Reply::Answered(json!({ "data": { "hour": null } }))
                    }
                    _ => panic!("Unexpected product {product}"),
                }
            },
            &mut transcript,
        ).unwrap();
        assert_eq!(reached.len(), 3000);
        assert_eq!(navigated.len(), 4);
        assert!(recorded[0].asked >= 2 * reached.len());
        assert_eq!(recorded[0].refused, 0);
        assert_eq!(recorded[0].pending, 0);
    }
}

#[test]
fn equipment_loss_plan_preserves_1500_tags_with_three_successive_holders() {
    let mut branch = branch();
    branch.last_hour = 899;
    branch.windows = vec![(0, 899)];
    let countries: serde_json::Map<String, Value> = (0..1500u32)
        .map(|tag| {
            let identities: Vec<_> = (0..3u32)
                .map(|at| {
                    json!({
                        "slot": tag, "countryId": tag + at * 1500,
                        "firstHour": at * 300, "lastHour": at * 300 + 299,
                    })
                })
                .collect();
            (format!("C{tag:04}"), json!({ "identities": identities }))
        })
        .collect();
    let mut swept = HashSet::new();
    let recorded = record(
        "equipment_losses",
        "1",
        &[Owner::Plan(
            "equipment-losses",
            &crate::products::equipment_losses::served::PLAN,
        )],
        &[branch],
        &mut |product, params| {
            if product == "country-identities" {
                return Reply::Answered(json!({ "data": { "countries": countries } }));
            }
            match product {
                "equipment-losses" => {
                    let selection = &params["selection"];
                    let country = &selection["country"];
                    let id = country["countryId"].as_u64().unwrap();
                    assert_eq!(country["slot"], id % 1500);
                    assert_eq!(country["originHour"], id / 1500 * 300);
                    if params["from"] == 0
                        && params["to"] == 899
                        && params["headHour"] == 899
                        && params["detailHour"] == 899
                    {
                        swept.insert((
                            selection["tag"].as_str().unwrap().to_owned(),
                            id,
                            selection["landSource"].as_str().unwrap().to_owned(),
                        ));
                    }
                    Reply::Answered(json!({ "data": { "causes": [], "designs": [] } }))
                }
                "series-next-change" => Reply::Answered(json!({ "data": { "hour": null } })),
                _ => panic!("Unexpected product {product}"),
            }
        },
        &mut io::sink(),
    )
    .unwrap();
    assert_eq!(swept.len(), 1500 * 3 * 2);
    assert!(recorded[0].asked >= swept.len());
    assert!(recorded[0].asked > PER_BRANCH);
    assert_eq!(recorded[0].answered, recorded[0].asked);
    assert_eq!((recorded[0].pending, recorded[0].refused), (0, 0));
}

#[test]
fn equipment_loss_plan_samples_naval_and_combat_log_activity_separately() {
    let countries: serde_json::Map<String, Value> = ["A".to_owned(), "B".to_owned()]
        .into_iter()
        .chain((0..15).map(|at| format!("C{at:02}")))
        .enumerate()
        .map(|(id, tag)| {
            (
                tag,
                json!({ "identities": [{
            "slot": id, "countryId": id, "firstHour": 0, "lastHour": 3,
        }] }),
            )
        })
        .collect();
    let mut transcript = Vec::new();
    let recorded = record(
        "equipment_losses",
        "1",
        &[Owner::Plan("equipment-losses", &crate::products::equipment_losses::served::PLAN)],
        &[branch()],
        &mut |product, params| {
            if product == "country-identities" {
                return Reply::Answered(json!({ "data": { "countries": countries } }));
            }
            if product == "series-next-change" {
                return Reply::Answered(json!({ "data": { "hour": 1 } }));
            }
            assert_eq!(product, "equipment-losses");
            let selection = &params["selection"];
            let source = if selection["tag"] == "B" {
                (selection["landSource"] == "combatLog")
                    .then(|| json!({ "kind": "combatLog", "reason": 0 }))
            } else {
                Some(json!({ "kind": "sunkShips" }))
            };
            let causes: Vec<_> = source.into_iter().map(|source| json!({
                "source": source, "domain": if selection["tag"] == "B" { "land" } else { "naval" },
                "name": { "key": null, "template": null, "text": null }, "group": null,
                "loss": true, "unknown": null, "unknownSpans": [],
                "leaves": [{ "design": 0, "points": [{ "hour": 1, "totalRaw": "5" }], "lowerBoundFrom": null }],
            })).collect();
            Reply::Answered(json!({ "data": { "causes": causes, "designs": [{ "variant": 99 }] } }))
        },
        &mut transcript,
    ).unwrap();
    assert_eq!(recorded[0].answered, recorded[0].asked);
    let mut lines = transcript.as_slice();
    read_header(&mut lines).unwrap();
    let exchanges: Vec<Exchange> = lines
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect();
    for (tag, source, kind) in [
        ("A", "destruction", "sunkShips"),
        ("B", "combatLog", "combatLog"),
    ] {
        let details: Vec<_> = exchanges
            .iter()
            .filter(|exchange| {
                exchange.product == "equipment-losses"
                    && exchange.params["selection"]["tag"] == tag
                    && exchange.params["selection"]["landSource"] == source
            })
            .map(|exchange| &exchange.params)
            .collect();
        assert!(
            details
                .iter()
                .any(|params| params["headHour"] == 3 && params["detailHour"] == 0),
            "{tag}: closed cards"
        );
        assert!(
            details.iter().any(|params| params["from"] == 0
                && params["to"] == 3
                && params["detailHour"] == 1),
            "{tag}: chart viewport"
        );
        assert!(
            details.iter().any(|params| params["width"] == 1
                && params["headHour"] == 2
                && params["detailHour"] == 1),
            "{tag}: reached dates"
        );
        let navigation: Vec<_> = exchanges
            .iter()
            .filter(|exchange| {
                exchange.product == "series-next-change"
                    && exchange.params["family"]["selection"]["tag"] == tag
                    && exchange.params["family"]["selection"]["landSource"] == source
            })
            .map(|exchange| &exchange.params)
            .collect();
        for key in ["Home", "End", "ArrowLeft", "ArrowRight"] {
            assert!(
                navigation
                    .iter()
                    .any(|params| params["key"] == key && params["leaves"][0][0]["kind"] == kind),
                "{tag}: {key}"
            );
        }
    }
    let mut lines = transcript.as_slice();
    read_header(&mut lines).unwrap();
    let report = check(&mut lines, &mut |product, params| {
        if product == "equipment-losses"
            && params["selection"]["tag"] == "B"
            && params["selection"]["landSource"] == "combatLog"
        {
            Reply::Answered(json!({ "data": { "causes": [], "designs": [] } }))
        } else {
            exchanges
                .iter()
                .find(|exchange| exchange.product == product && exchange.params == *params)
                .unwrap()
                .reply
                .clone()
        }
    })
    .unwrap();
    assert!(report.differing > 0);
}

#[test]
fn declared_request_allowances_still_reject_unbounded_plans() {
    static LIMITED: Plan = Plan {
        queries: limited,
        unordered: &[],
    };
    fn limited(ask: &mut dyn Ask) -> io::Result<()> {
        ask.request_allowance(1, 2)?;
        for at in 0..3 {
            ask.ask("toy", json!({ "at": at }))?;
        }
        Ok(())
    }
    let error = record(
        "toy",
        "1",
        &[Owner::Plan("toy", &LIMITED)],
        &[branch()],
        &mut |_, _| Reply::Answered(json!({ "data": {} })),
        &mut io::sink(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "toy asked more than 2 requests");
}

#[test]
fn request_allowances_are_scoped_to_the_declaring_plan() {
    static SMALL: Plan = Plan {
        queries: small,
        unordered: &[],
    };
    static DEFAULT: Plan = Plan {
        queries: default,
        unordered: &[],
    };
    fn small(ask: &mut dyn Ask) -> io::Result<()> {
        ask.request_allowance(1, 1)?;
        ask.ask("small", json!({}))?;
        Ok(())
    }
    fn default(ask: &mut dyn Ask) -> io::Result<()> {
        for at in 0..PER_BRANCH {
            ask.ask("default", json!({ "at": at }))?;
        }
        Ok(())
    }
    let recorded = record(
        "toy",
        "1",
        &[
            Owner::Plan("small", &SMALL),
            Owner::Plan("default", &DEFAULT),
        ],
        &[branch()],
        &mut |_, _| Reply::Answered(json!({ "data": {} })),
        &mut io::sink(),
    )
    .unwrap();
    assert_eq!(recorded[0].asked, 1);
    assert_eq!(recorded[1].asked, PER_BRANCH);
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
fn an_ignored_path_is_passed_over_and_only_it() {
    let path = |text: &str| text.split('.').map(str::to_owned).collect::<Vec<_>>();
    let ignored = [path("data.pages.*.firstHour")];
    let at = |reference: Value, candidate: Value| {
        difference_ignoring(
            &Reply::Answered(json!({ "data": reference })),
            &Reply::Answered(json!({ "data": candidate })),
            &[],
            &ignored,
        )
    };
    let page = |first: i32, value: i32| json!({ "firstHour": first, "value": value });
    assert_eq!(
        at(
            json!({ "pages": [page(1, 7)] }),
            json!({ "pages": [page(3, 7)] })
        ),
        None
    );
    assert!(
        at(
            json!({ "pages": [page(1, 7)] }),
            json!({ "pages": [page(1, 8)] })
        )
        .is_some()
    );
    assert!(at(json!({ "firstHour": 1 }), json!({ "firstHour": 3 })).is_some());
    assert!(at(json!({ "pages": [page(1, 7)] }), json!({ "pages": [] })).is_some());
}

#[test]
fn an_ignored_path_is_passed_over_where_absent_and_inside_unordered_arrays() {
    let path = |text: &str| text.split('.').map(str::to_owned).collect::<Vec<_>>();
    let unordered = [vec!["series".to_owned()]];
    let ignored = [path("data.firstHour"), path("data.series.*.firstHour")];
    let at = |reference: Value, candidate: Value| {
        difference_ignoring(
            &Reply::Answered(json!({ "data": reference })),
            &Reply::Answered(json!({ "data": candidate })),
            &unordered,
            &ignored,
        )
    };
    assert_eq!(
        at(json!({ "firstHour": 1, "v": 2 }), json!({ "v": 2 })),
        None
    );
    assert!(at(json!({ "firstHour": 1, "v": 2 }), json!({ "v": 3 })).is_some());
    let run = |first: i32, value: i32| json!({ "firstHour": first, "value": value });
    assert_eq!(
        at(
            json!({ "series": [run(1, 7), run(2, 8)] }),
            json!({ "series": [run(5, 8), run(6, 7)] })
        ),
        None
    );
    assert!(
        at(
            json!({ "series": [run(1, 7), run(2, 7)] }),
            json!({ "series": [run(1, 7), run(2, 8)] })
        )
        .is_some()
    );
}

#[test]
fn ignoring_an_unordered_arrays_elements_passes_over_them_all() {
    let unordered = [vec!["series".to_owned()]];
    let ignored = [vec!["data".to_owned(), "series".to_owned(), "*".to_owned()]];
    let at = |reference: Value, candidate: Value| {
        difference_ignoring(
            &Reply::Answered(json!({ "data": reference })),
            &Reply::Answered(json!({ "data": candidate })),
            &unordered,
            &ignored,
        )
    };
    assert_eq!(
        at(json!({ "series": [1, 2] }), json!({ "series": [3] })),
        None
    );
    assert_eq!(at(json!({ "series": [1] }), json!({ "series": [] })), None);
    assert!(
        at(
            json!({ "series": [1], "v": 1 }),
            json!({ "series": [], "v": 2 })
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
        "casualties",
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
