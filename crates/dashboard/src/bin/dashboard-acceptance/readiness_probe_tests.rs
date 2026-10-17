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
    probe.record(index, Ok(json!({"data": data}).to_string()), 0.0, 0.0);
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

#[test]
fn terminal_states_reask_an_empty_gate_once_and_settle_null_as_unavailable() {
    with_probe(|probe| {
        for state in ["ready", "paused", "failed", "cancelled", "idle"] {
            let template = readiness::Template {
                product: "army-size".into(),
                route: Route::Prepared,
                params: json!({"branchId":"1", "testState":state}),
                first_view: None,
            };
            let index = probe.track(&template, template.params.clone(), false);
            let preparing = json!({"state":"preparing", "families":{"army":{}}, "sections":[]});
            {
                let tracked = &mut probe.tracked[index];
                tracked.queries = 2;
                tracked.asked_status = preparing.clone();
            }
            let reply = || Ok(json!({"data":null,"gate":["army"]}).to_string());
            probe.record(index, reply(), 0.0, 0.0);
            assert_eq!(probe.tracked[index].outcome, Some(Outcome::Pending));
            assert!(!probe.tracked[index].due(&preparing, 500.0));
            probe.status = preparing.clone();
            probe.status["state"] = json!(state);
            // The coverage/count key is unchanged; only the terminal state moved.
            assert_eq!(
                progress_key(&preparing, &template.params, Some(&["army".into()])),
                progress_key(&probe.status, &template.params, Some(&["army".into()]))
            );
            assert!(
                probe.tracked[index].due(&probe.status, 500.0),
                "{state} lost its final ask"
            );
            probe.ask(index);
            let asked = probe.tracked[index].in_flight.take().unwrap();
            probe.record(index, reply(), asked.at_ms, 0.0);
            assert_eq!(probe.tracked[index].outcome, Some(Outcome::Unavailable));
            assert!(probe.tracked[index].exact_ms.is_some());
            assert!(!probe.tracked[index].due(&probe.status, 501.0));
        }
    });
}

#[test]
fn a_terminal_final_ask_retains_an_unfinished_prefix_and_stops_refreshing() {
    with_probe(|probe| {
        let template = readiness::Template {
            product: "army-size".into(),
            route: Route::Prepared,
            params: json!({"branchId":"1"}),
            first_view: None,
        };
        let index = probe.track(&template, template.params.clone(), true);
        let extent = |last| json!({"firstHour":0,"lastHour":last,"complete":false});
        let preparing = json!({"state":"preparing","families":{"army":{"1":extent(9)}}});
        probe.tracked[index].queries = 2;
        probe.tracked[index].asked_status = preparing.clone();
        probe.record(
            index,
            Ok(json!({"gate":["army"],"data":{
            "coverage":extent(9),"pending":[{"firstHour":10,"lastHour":29}]}})
            .to_string()),
            0.0,
            0.0,
        );
        probe.tracked[index].refresh.replied(1000.0, false);
        probe.status = json!({"state":"failed","error":"Stopped after commit",
            "families":{"army":{"1":extent(20)}}});
        assert!(
            probe.tracked[index].due(&probe.status, 1001.0),
            "terminal read was throttled"
        );
        probe.ask(index);
        let asked = probe.tracked[index].in_flight.take().unwrap();
        probe.record(
            index,
            Ok(json!({"gate":["army"],"data":{
            "coverage":extent(20),"pending":[{"firstHour":21,"lastHour":29}]}})
            .to_string()),
            asked.at_ms,
            0.0,
        );
        let tracked = &probe.tracked[index];
        assert_eq!(tracked.outcome, Some(Outcome::Prefix));
        assert_eq!(tracked.data.as_ref().unwrap()["coverage"]["lastHour"], 20);
        assert_eq!(tracked.error.as_deref(), Some("Stopped after commit"));
        assert!(
            tracked.exact_ms.is_none(),
            "a terminal prefix was presented as exact"
        );
        assert!(
            !tracked.due(&probe.status, 10000.0),
            "a stopped request kept polling"
        );
    });
}

#[test]
fn terminal_drain_finalizes_a_delayed_map_including_a_map_only_table() {
    for map_only in [false, true] {
        with_probe(|probe| {
            let entries = readiness::widgets(
                r#"{"widgets":[
                    {"kind":"map","map":true},
                    {"kind":"army-size","requests":[
                        {"product":"army-size","route":"prepared","params":{"branchId":"1"}}
                    ]}
                ]}"#,
            )
            .unwrap();
            let mut widgets: Vec<_> = entries.into_iter().map(widget).collect();
            if map_only {
                widgets.truncate(1);
            }
            let (release, delayed) = std::sync::mpsc::channel();
            let thread = std::thread::spawn(move || {
                delayed.recv().unwrap();
                Ok(json!({"event":"map", "readyMs":1234.0, "frame":true}))
            });
            probe.status = json!({"state":"failed"});
            for widget in widgets.iter_mut().filter(|widget| !widget.entry.map) {
                widget.refresh(probe, &bindings()).unwrap();
                let index = request(widget, 0);
                answer(probe, index, json!({"exact":1}));
                widget.ready_ms = settled(probe, &widget.slots).map(|(at, _)| at);
                widget.exact_ms = exact(probe, &widget.slots).map(|(at, _)| at);
                assert!(widget.exact_ms.is_some());
            }
            assert!(stopped(&probe.status));
            assert!(probe.tracked.iter().all(|tracked| {
                tracked.in_flight.is_none() && !tracked.due(&probe.status, probe.now_ms())
            }));
            assert!(!thread.is_finished(), "map must outlast the terminal drain");
            assert!(widgets[0].ready_ms.is_none());
            assert!(widgets[0].exact_ms.is_none());

            // Polling has stopped. The final join must publish the same widget
            // milestones as collection inside the loop, without another poll.
            release.send(()).unwrap();
            let mut lines = Vec::new();
            let map_ms = finish_map(thread, &mut widgets, |line| {
                lines.push(line);
                Ok(())
            })
            .unwrap();
            assert_eq!(map_ms, Some(1234.0));
            assert_eq!(widgets[0].ready_ms, map_ms);
            assert_eq!(widgets[0].exact_ms, map_ms);
            assert!(widgets.iter().all(|widget| widget.ready_ms.is_some()));
            assert!(widgets.iter().all(|widget| widget.exact_ms.is_some()));
            assert_eq!(lines.len(), 3);
            assert_eq!(lines[0]["event"], "map");
            assert_eq!(lines[1]["event"], "widget");
            assert_eq!(lines[1]["kind"], "map");
            assert_eq!(lines[1]["readyMs"], 1234.0);
            assert_eq!(lines[2]["event"], "widget-exact");
            assert_eq!(lines[2]["kind"], "map");
            assert_eq!(lines[2]["exactMs"], 1234.0);
            // The first-view CPU record follows the join, once.
            let mut emitted = false;
            for _ in 0..2 {
                ready_milestone(
                    probe,
                    &probe.status.clone(),
                    &widgets,
                    &mut emitted,
                    |line| {
                        lines.push(line);
                        Ok(())
                    },
                )
                .unwrap();
            }
            assert_eq!(lines.len(), 4);
            assert_eq!(lines[3]["event"], "cpu");
            assert_eq!(lines[3]["allReady"], true);
        });
    }
}
