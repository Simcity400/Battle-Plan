use super::*;

fn with_probe(run: impl FnOnce(&mut Probe)) {
    let service = Service::new(Arc::new(|_| true)).unwrap();
    let channel = Channel::default();
    let mut probe = Probe {
        service: &service,
        channel: &channel,
        lease: "dependency-test".into(),
        revision: "1".into(),
        started: Instant::now(),
        next: 0,
        epoch: 0,
        tracked: Vec::new(),
        by_key: HashMap::new(),
        status_queries: 0,
        status_ms: 0.0,
        lines: Vec::new(),
        status: Value::Null,
    };
    run(&mut probe);
    drop(probe);
    service.shutdown();
}

fn bindings() -> Bindings {
    Bindings {
        branch: "1".into(),
        tag: "GER".into(),
        first: 0,
        last: 29,
        width: CHART_WIDTH,
    }
}

fn widget(entry: readiness::Widget) -> WidgetState {
    WidgetState {
        slots: vec![Slot::Unresolved; entry.requests.len()],
        entry,
        auxiliary: Vec::new(),
        ready_ms: None,
        exact_ms: None,
    }
}

fn request(widget: &WidgetState, slot: usize) -> usize {
    match widget.slots[slot] {
        Slot::Request(index) => index,
        _ => panic!("slot {slot} has no request"),
    }
}

fn answer(probe: &mut Probe, index: usize, data: Value) {
    probe.record(
        index,
        Ok(json!({"data": data}).to_string()),
        false,
        0.0,
        0.0,
    );
}

fn groups(countries: &[u32], complete: bool) -> Value {
    json!({
        "countryIdentities": [{"tag": "GER", "identities": countries.iter()
            .map(|id| json!({"slot": 7, "countryId": id})).collect::<Vec<_>>()}],
        "coverage": {"firstHour": 0, "lastHour": if complete {29} else {-1}, "complete": complete},
        "approximate": if complete {json!([])} else {json!([{"firstHour":0,"lastHour":29}])},
        "pending": [],
    })
}

#[test]
fn production_lines_follows_its_first_view_with_the_widgets_exact_request() {
    with_probe(|probe| {
        let entry = readiness::widgets(readiness::WIDGETS)
            .unwrap()
            .into_iter()
            .find(|entry| entry.kind == "production-lines")
            .unwrap();
        let mut widget = widget(entry);
        widget.refresh(probe, &bindings()).unwrap();
        let first = request(&widget, 0);
        let data = json!({"approximate": [{"firstHour": 0, "lastHour": 15}]});
        answer(probe, first, data.clone());
        let first_view = settled(probe, &widget.slots).unwrap();
        assert!(exact(probe, &widget.slots).is_none());
        let follow_up = probe.tracked[first]
            .exact_request
            .expect("whole history follow-up");
        assert_eq!(probe.tracked[follow_up].route, Route::Product);
        let mut expected = probe.tracked[first].params.clone();
        expected["exact"] = json!(true);
        assert_eq!(probe.tracked[follow_up].params, expected);
        answer(probe, first, data);
        assert_eq!(probe.tracked[first].exact_request, Some(follow_up));
        assert_eq!(probe.tracked.len(), 2, "one shared follow-up");
        answer(probe, follow_up, json!({"approximate": []}));
        assert_eq!(settled(probe, &widget.slots).unwrap(), first_view);
        assert_eq!(exact(probe, &widget.slots).unwrap().1, Some(follow_up));
        assert_eq!(probe.tracked.len(), 2, "an exact reply needs no follow-up");
    });
}

#[test]
fn experience_exactness_waits_for_requests_from_the_final_holder_history() {
    with_probe(|probe| {
        let entry = readiness::widgets(readiness::WIDGETS)
            .unwrap()
            .into_iter()
            .find(|entry| entry.kind == "experience")
            .unwrap();
        let mut widget = widget(entry);
        let bindings = bindings();
        widget.refresh(probe, &bindings).unwrap();
        let dependency = request(&widget, 0);
        // The cold keyframes miss a holder introduced inside a block.
        answer(probe, dependency, groups(&[9001], false));
        widget.refresh(probe, &bindings).unwrap();
        let initial: Vec<_> = (1..widget.slots.len())
            .map(|slot| request(&widget, slot))
            .collect();
        for &index in &initial {
            answer(probe, index, json!({"states": []}));
        }
        assert!(settled(probe, &widget.slots).is_some());
        assert!(exact(probe, &widget.slots).is_none());

        let final_groups = groups(&[9001, 9002], true);
        answer(probe, dependency, final_groups.clone());
        assert_eq!(probe.tracked[dependency].data.as_ref(), Some(&final_groups));
        widget.refresh(probe, &bindings).unwrap();
        assert!(exact(probe, &widget.slots).is_none());
        for (offset, &before) in initial.iter().enumerate() {
            let slot = offset + 1;
            let index = request(&widget, slot);
            assert_ne!(index, before);
            let params = &probe.tracked[index].params;
            let keys: Vec<_> = if params["tags"].is_array() {
                params["tags"].as_array().unwrap().clone()
            } else {
                params["reads"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|read| read["tag"].clone())
                    .collect()
            };
            assert_eq!(
                keys,
                [json!(r#"["GER",7,9001]"#), json!(r#"["GER",7,9002]"#)]
            );
            answer(probe, index, json!({"states": []}));
        }
        assert!(exact(probe, &widget.slots).is_some());
        let tracked = probe.tracked.len();
        widget.refresh(probe, &bindings).unwrap();
        assert_eq!(
            probe.tracked.len(),
            tracked,
            "unchanged selections share requests"
        );
    });
}

#[test]
fn a_window_absent_from_a_sampled_manifest_is_resolved_again() {
    with_probe(|probe| {
        let entry = readiness::widgets(
            r#"{"widgets":[{"kind":"windows","requests":[
                {"product":"land-battles-manifest","route":"prepared","params":{"branchId":"$branch"}},
                {"product":"land-battles","route":"prepared","params":{"branchId":"$branch","window":"$window:0:last"}}
            ]}]}"#,
        )
        .unwrap()
        .remove(0);
        let mut widget = widget(entry);
        let bindings = bindings();
        widget.refresh(probe, &bindings).unwrap();
        let dependency = request(&widget, 0);
        answer(
            probe,
            dependency,
            json!({
                "windows": [], "coverage": {"firstHour":0,"lastHour":-1,"complete":false},
                "pending": [{"firstHour":0,"lastHour":29}],
            }),
        );
        widget.refresh(probe, &bindings).unwrap();
        assert!(matches!(widget.slots[1], Slot::Absent(_)));
        assert!(settled(probe, &widget.slots).is_some());
        assert!(exact(probe, &widget.slots).is_none());

        answer(
            probe,
            dependency,
            json!({
                "windows": [{"firstHour":0,"lastHour":29}],
                "coverage": {"firstHour":0,"lastHour":29,"complete":true},
            }),
        );
        widget.refresh(probe, &bindings).unwrap();
        let window = request(&widget, 1);
        assert_eq!(probe.tracked[window].params["window"], 0);
        assert!(exact(probe, &widget.slots).is_none());
        answer(probe, window, json!({"exact": 0}));
        assert!(exact(probe, &widget.slots).is_some());
    });
}
