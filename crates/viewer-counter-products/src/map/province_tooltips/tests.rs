use super::*;

#[test]
fn air_region_hover_rebuilds_for_each_views_ordered_wing_selection() {
    let replay = Arc::new(air_region::Sampler::today(air_region::tests::recording(80, &[]), None));
    let mut holder = Holder::<ModeHover>::default();
    let mut built = 0;
    let mut ask = |holder: &mut Holder<ModeHover>, selected: &[u64]| {
        holder.get(
            ModeHover::StrategicAir(Arc::new(selected.to_vec())),
            true,
            &replay,
            || {},
            |_| {
                built += 1;
                Tooltip {
                    title: built.to_string(),
                    ..Default::default()
                }
            },
        )
    };
    let first = ask(&mut holder, &[10, 20]);
    let same = ask(&mut holder, &[10, 20]);
    assert!(Arc::ptr_eq(&first, &same));
    let reordered = ask(&mut holder, &[20, 10]);
    assert_eq!(reordered.title, "2");
    let cleared = ask(&mut holder, &[]);
    assert_eq!(cleared.title, "3");
    assert!(!Arc::ptr_eq(&first, &cleared));
}

#[test]
fn thermal_identifier_notes_reach_province_content_warnings() {
    let content = Content {
        labels: Arc::new(
            viewer_runtime::recorded_labels::RecordedLabels::from_localisation(
                &Default::default(),
            ),
        ),
        members: Default::default(),
        impassable: Default::default(),
        neighbors: Ok(Default::default()),
        temperature: Ok(content_reader_weather::weather::TemperatureRanges {
            ranges: Default::default(),
            notes: vec!["fixture modifier: unsupported identifier field (1 modifier)".into()],
        }),
        pass: Err("unread".into()),
        derived: Default::default(),
        units: Err("unused".into()),
        border: Err("unused".into()),
        air_log_timeout: Err("unused".into()),
        air_detection: Err("unused".into()),
        air_range: Err("unused".into()),
        air_fuel: Err("unused".into()),
        navy: super::strategic_navy::Defines::default(),
        occupation: Err("unused".into()),
        network_base_target: Err("unused".into()),
        resources: Err("unused".into()),
    };
    assert!(content.warnings().iter().any(|warning| warning ==
        "fixture modifier: unsupported identifier field (1 modifier)"));
    assert!(
        content.warnings().iter().any(|warning| warning.contains(
            "Province temperatures cannot be derived from the recorded content (unread)"
        ))
    );
}

#[test]
fn native_multiline_indentation_keeps_gaps_and_refusals_out_of_text() {
    let mut tooltip = Tooltip::default();
    line(
        &mut tooltip,
        "location",
        0,
        Ok("Forest in Test state".into()),
    );
    line(&mut tooltip, "owner", 1, Ok("Owner\n\nController".into()));
    line(
        &mut tooltip,
        "missing input",
        1,
        Err("operand unavailable".into()),
    );
    line(&mut tooltip, "vp", 1, Ok("Victory point: 1".into()));
    assert_eq!(
        tooltip.title,
        "Forest in Test state\n    Owner\n\n    Controller\n    Victory point: 1"
    );
    assert_eq!(
        tooltip.refused["province missing input: operand unavailable"],
        1
    );
    assert!(tooltip.body.is_empty());
}

#[test]
fn a_tooltip_built_with_a_pending_history_rebuilds_once_answered() {
    let replay = Arc::new(air_region::Sampler::today(air_region::tests::recording(80, &[]), None));
    let mut holder = Holder::<u32>::default();
    let built = std::sync::atomic::AtomicUsize::new(0);
    let ask = |holder: &mut Holder<u32>| {
        holder.get(
            7,
            true,
            &replay,
            || {},
            |rosters| {
                built.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let mut out = Tooltip::default();
                match rosters.visiting_pass(1, 127, 5) {
                    Some(Ok(pass)) => out.title = format!("pass {pass:?}"),
                    Some(Err(reason)) => out.refuse("pass", reason),
                    None => {}
                }
                out
            },
        )
    };
    let first = ask(&mut holder);
    assert!(first.title.is_empty());
    let pending = holder.pending();
    if pending {
        holder.wait();
    }
    let answered = ask(&mut holder);
    assert_eq!(answered.title, "pass Some(72)");
    assert!(!holder.pending());
    assert!(pending);
    assert_eq!(built.load(std::sync::atomic::Ordering::Relaxed), 2);
    let held = ask(&mut holder);
    assert!(Arc::ptr_eq(&held, &answered));
    assert_eq!(built.load(std::sync::atomic::Ordering::Relaxed), 2);
    holder.wait();
}

#[test]
fn navy_hover_rebuilds_when_prepared_source_coverage_finishes_without_an_hour_change() {
    let replay = Arc::new(air_region::Sampler::today(air_region::tests::recording(80, &[]), None));
    let mut holder = Holder::<ModeHover>::default();
    let mut built = 0;
    let mut ask = |holder: &mut Holder<ModeHover>, sources| {
        holder.get(
            ModeHover::StrategicNavy(Arc::new(sources)),
            true,
            &replay,
            || {},
            |_| {
                built += 1;
                Tooltip {
                    title: built.to_string(),
                    ..Default::default()
                }
            },
        )
    };
    let first = ask(&mut holder, Err("pending sources".into()));
    let same = ask(&mut holder, Err("pending sources".into()));
    assert!(Arc::ptr_eq(&first, &same));
    let ready = ask(&mut holder, Ok(Default::default()));
    assert_eq!(ready.title, "2");
    let changed = ask(
        &mut holder,
        Ok([(17, Err("selected ship design refused".into()))].into()),
    );
    assert_eq!(changed.title, "3");
}

#[test]
fn a_land_province_gives_the_delayed_click_footer_and_water_does_not() {
    use content_reader_map::map::ProvinceKind;
    let loc: Localisation = [("PROVINCE_CLICK_VIEW", "Click to view State")]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    let footer = |kind| {
        let mut out = Tooltip { body: "selection".into(), ..Default::default() };
        click_footer(&loc, kind, &mut out);
        out
    };
    assert_eq!(footer(Some(ProvinceKind::Land)).body, "selection
Click to view State");
    for water in [ProvinceKind::Sea, ProvinceKind::Lake] {
        let out = footer(Some(water));
        assert_eq!(out.body, "selection");
        assert!(out.refused.is_empty());
    }
    assert_eq!(footer(None).refused.len(), 1);
}
