use super::*;
use crate::map::scene_failure_tests::Fixture;
use viewer_gpu::camera::Placement;
use viewer_runtime::test_support::TEST_HANG_GUARD;
use viewer_map_modes::map_mode::MapMode;

const MAP: [f32; 2] = [640., 480.];

fn target(fixture: &Fixture, id: &str, x: i32) -> dashboard_core::protocol::DashboardMapView {
    let bounds = dashboard_core::protocol::ViewBounds {
        x,
        y: 0,
        width: MAP[0] as u32,
        height: MAP[1] as u32,
    };
    dashboard_core::protocol::DashboardMapView {
        options: Default::default(),
        id: id.into(),
        source_id: "verified-fixture".into(),
        path: "fixture".into(),
        revision: "fixture".into(),
        branch_id: fixture.input.branch.to_string(),
        playing: false,
        hours_per_second: 1.0,
        hour: fixture.input.hour,
        fraction: 0.,
        sequence: 1,
        seek_sequence: Some(1),
        composed_at: 0.0,
        clock_id: "page-1".into(),
        bounds,
        clip: bounds,
        visible: true,
    }
}

fn left(index: usize) -> f32 {
    index as f32 * MAP[0]
}

fn pass(
    viewers: &mut [Viewer; 2],
    targets: &[dashboard_core::protocol::DashboardMapView; 2],
    ctx: &egui::Context,
    frame: &eframe::Frame,
    events: Vec<egui::Event>,
) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(MAP[0] * 2., MAP[1]),
            )),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            for (index, (viewer, target)) in viewers.iter_mut().zip(targets).enumerate() {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(left(index), 0.),
                    egui::vec2(MAP[0], MAP[1]),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    viewer.map_ui(ui, frame, target)
                });
            }
        },
    );
    output.textures_delta.clear();
}

fn pointer(x: f32, y: f32) -> egui::Event {
    egui::Event::PointerMoved(egui::pos2(x, y))
}

fn key((modifiers, key): (egui::Modifiers, egui::Key)) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn press(x: f32, y: f32, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: egui::pos2(x, y),
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn the_picker_keys_and_toggles_reach_one_map() -> std::io::Result<()> {
    let fixture = Fixture::load("viewer-picker-input")?;
    let request = fixture.request([MAP[0] as u32, MAP[1] as u32]);
    let viewer = || -> std::io::Result<Viewer> {
        let document = Document::from_shared_content(
            fixture.document.content.clone(),
            fixture.document.playback.replay_handle(),
            &InitialHour {
                branch: Some(fixture.input.branch),
                hour: Some(fixture.input.hour),
                hour_fraction: 0.,
            },
        )?;
        let mut viewer = Viewer::create(
            Config::default(),
            Options::default(),
            None,
            Arc::new(Mutex::new(None)),
            None,
            Some((fixture.state.device.clone(), fixture.state.queue.clone())),
        );
        viewer.dashboard_map = true;
        viewer.diagnostics = false;
        viewer.graphics.samples = 1;
        viewer.ui_scale = Setting::Manual(1.0);
        viewer.camera_manual = true;
        viewer.camera = Some(Placement::map(request.camera.center, request.camera.height));
        viewer.install_map(document, fixture.scene()?, "fixture".into());
        Ok(viewer)
    };
    let mut viewers = [viewer()?, viewer()?];
    let targets = [
        target(&fixture, "first", 0),
        target(&fixture, "second", MAP[0] as i32),
    ];
    let ctx = egui::Context::default();
    let mut frame = eframe::Frame::_new_kittest();
    frame.wgpu_render_state = Some(fixture.state.clone());
    let deadline = Instant::now() + TEST_HANG_GUARD;
    while !viewers.iter().all(|v| v.rendered.is_some()) {
        pass(&mut viewers, &targets, &ctx, &frame, Vec::new());
        for viewer in &viewers {
            assert!(
                viewer.error.is_none() && viewer.map_error.is_none(),
                "{:?} {:?}",
                viewer.error,
                viewer.map_error
            );
        }
        assert!(Instant::now() < deadline, "the maps did not draw");
        std::thread::sleep(Duration::from_millis(1));
    }

    let document = viewers[0].document.as_ref().unwrap();
    let slots = super::picker::slot_modes(document, &[]);
    let keys: Vec<_> = viewer_combat::picker::shortcuts(
        &document.combat,
        MAP,
        super::picker::four_buttons(document),
        super::picker::DEFAULT_SLOTS.len(),
    )?
    .into_iter()
    .filter_map(|(control, text)| match control {
        viewer_combat::picker::Control::Slot(slot)
            if slots[slot] != MapMode::default()
                && !matches!(&slots[slot], MapMode::Stock(stock) if stock.unavailable().is_some()) =>
        {
            Some((slots[slot].clone(), super::picker::parse_shortcut(&text)?))
        }
        _ => None,
    })
    .collect();
    assert!(
        keys.len() >= 2,
        "the picker authors fewer than two mode keys"
    );
    let (first_mode, first_key) = keys[0].clone();
    let (second_mode, second_key) = keys[1].clone();

    let over_second = left(1) + MAP[0] / 2.;
    pass(
        &mut viewers,
        &targets,
        &ctx,
        &frame,
        vec![pointer(over_second, MAP[1] / 2.), key(first_key)],
    );
    assert_eq!(viewers[0].map_mode, MapMode::default());
    assert_eq!(viewers[1].map_mode, first_mode);

    let over_first = MAP[0] / 2.;
    pass(
        &mut viewers,
        &targets,
        &ctx,
        &frame,
        vec![
            pointer(over_first, MAP[1] / 2.),
            press(over_first, MAP[1] / 2., true),
        ],
    );
    pass(
        &mut viewers,
        &targets,
        &ctx,
        &frame,
        vec![press(over_first, MAP[1] / 2., false)],
    );
    pass(
        &mut viewers,
        &targets,
        &ctx,
        &frame,
        vec![pointer(over_second, MAP[1] / 2.), key(second_key)],
    );
    assert_eq!(viewers[0].map_mode, second_mode);
    assert_eq!(viewers[1].map_mode, first_mode);

    viewers[1].picker.open = viewer_combat::picker::Open::All;
    pass(&mut viewers, &targets, &ctx, &frame, Vec::new());
    let entries = |viewer: &mut Viewer| {
        let document = viewer.document.take().unwrap();
        let drawn = viewer
            .draw_picker(&document, MAP, false, 0., false, None)
            .unwrap()
            .unwrap();
        let modes = super::picker::all_modes(&document, super::picker::four_buttons(&document));
        viewer.document = Some(document);
        let shown: Vec<_> = drawn
            .controls
            .iter()
            .filter(|placed| matches!(placed.control, viewer_combat::picker::Control::Entry(_)))
            .map(|placed| (placed.rect, placed.control.clone()))
            .collect();
        let windows: Vec<_> = drawn
            .controls
            .iter()
            .filter(|placed| placed.control == viewer_combat::picker::Control::Window)
            .map(|placed| placed.rect)
            .collect();
        (shown, windows, modes.len())
    };
    let (before, windows, modes) = entries(&mut viewers[1]);
    assert_eq!(before.len(), modes, "every mode has its entry");
    for (rect, control) in &before {
        assert!(
            windows.iter().any(|window| {
                rect[0] >= window[0]
                    && rect[1] >= window[1]
                    && rect[0] + rect[2] <= window[0] + window[2]
                    && rect[1] + rect[3] <= window[1] + window[3]
            }),
            "{control:?} at {rect:?} lies outside its window"
        );
    }
    let (x, y) = (before[0].0[0] + 2., before[0].0[1] + 2.);
    pass(
        &mut viewers,
        &targets,
        &ctx,
        &frame,
        vec![
            pointer(left(1) + x, y),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0., -1.),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_eq!(
        entries(&mut viewers[1]).0,
        before,
        "the wheel moved the mode list"
    );
    viewers[1].picker.open = viewer_combat::picker::Open::None;

    viewers[0].picker_toggle("player_counters_toggle");
    pass(&mut viewers, &targets, &ctx, &frame, Vec::new());
    let ours = viewers[0].overlay_tables.as_ref().expect("the side made the overlay's tables").ours();
    assert_eq!(
        viewers[0].overlay_layers.counters.owners.as_deref(),
        Some(&std::collections::BTreeSet::from([ours]))
    );
    assert_eq!(viewers[1].overlay_layers.counters.owners, None);
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn the_air_and_navy_modes_write_strategic_region_names() -> std::io::Result<()> {
    use viewer_map_modes::map_mode::Stock;
    let fixture = Fixture::load("viewer-region-names")?;
    let hour = fixture.document.hour()?;
    let none = viewer_map_text::name_pass::NameAreas::default();
    assert_ne!(hour.name_areas(&MapMode::default())?, none);
    for stock in [Stock::StrategicAir, Stock::StrategicNavy] {
        let areas = hour.name_areas(&MapMode::Stock(stock))?;
        assert_ne!(areas, none, "{stock:?}");
        let placed = hour.place_name_areas(&areas, &Default::default());
        assert!(!placed.country.is_empty(), "{stock:?}");
        assert!(
            placed
                .country
                .iter()
                .all(|name| name.key.starts_with("strategic region "))
        );
    }
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn the_states_mode_names_its_live_category_or_refuses_the_line() -> std::io::Result<()> {
    use viewer_counter_products::map::province_tooltips::{Cache, ModeHover};
    let fixture = Fixture::load("viewer-state-category")?;
    let document = &fixture.document;
    let viewer_playback::map_source::Current::Frame(frame) = document.playback.current() else {
        panic!("the fixture hour is a gap");
    };
    let province = frame
        .complete::<replay_data::rows::tables::Provinces>()
        .map_err(std::io::Error::other)?
        .iter()
        .find(|province| province.state != 0)
        .expect("a province in a state")
        .id;
    let vision = document.vision(&Default::default())?;
    let hover = |category| {
        Cache::default().request(
            &document.counter_content,
            document.icons.raids.movement_reads(),
            frame,
            province,
            &vision,
            false,
            &Arc::new(Vec::new()),
            &document.playback.roster_sampler(),
            &ModeHover::Location { category },
            &[],
            || {},
        )
    };
    let refusal = "province state category: the state's live category is not recorded";
    let states = hover(true);
    let default = hover(false);
    assert!(!default.refused.contains_key(refusal));
    if states.refused.get(refusal) == Some(&1) {
        assert_eq!(states.title, default.title);
    } else {
        assert!(
            states.title.len() > default.title.len(),
            "{:?}",
            states.refused
        );
    }
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn a_scripted_modes_hover_reads_the_viewer_and_the_hovered_state() -> std::io::Result<()> {
    use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, ScriptedText};
    use viewer_counter_products::map::province_tooltips::{Cache, ModeHover};
    let fixture = Fixture::load("viewer-scripted-hover")?;
    let document = &fixture.document;
    let viewer_playback::map_source::Current::Frame(frame) = document.playback.current() else {
        panic!("the fixture hour is a gap");
    };
    let province = frame
        .complete::<replay_data::rows::tables::Provinces>()
        .map_err(std::io::Error::other)?
        .iter()
        .find(|province| province.state != 0)
        .expect("a province in a state")
        .clone();
    let mut content = (*document.counter_content).clone();
    content.localisation = Arc::new(
        content
            .localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain([
                (
                    "fixture_mode_tooltip".to_owned(),
                    "[ROOT.GetName] - [FROM.GetName]".to_owned(),
                ),
                (
                    "fixture_mode_tooltip_delayed".to_owned(),
                    "[FROM.GetContinent]".to_owned(),
                ),
            ])
            .collect(),
    );
    let content = Arc::new(content);
    let vision = document.vision(&Default::default())?;
    let hover = Cache::default().request(
        &content,
        document.icons.raids.movement_reads(),
        frame,
        province.id,
        &vision,
        false,
        &Arc::new(Vec::new()),
        &document.playback.roster_sampler(),
        &ModeHover::Scripted("fixture_mode".into()),
        &[],
        || {},
    );
    let scripted = content.scripted.as_ref().as_ref().unwrap();
    let text = ScriptedText::new(frame, &content.localisation, scripted, vision.ours());
    let viewer = text.expand("[ROOT.GetName]").unwrap();
    let state = text
        .expand_in(
            "[THIS.GetName]",
            &EventScopes::new(vec![EventScope::of(EventObject::State(province.state))]),
        )
        .unwrap();
    assert_eq!(
        hover.title,
        format!("{viewer} - {state}"),
        "{:?}",
        hover.refused
    );
    assert!(
        hover
            .refused
            .keys()
            .any(|refusal| refusal.contains("GetContinent")),
        "{:?}",
        hover.refused
    );
    Ok(())
}

#[test]
#[ignore = "verified replay/content and GPU; requires BATTLEPLAN_SCENE_FAILURE_INPUT"]
fn the_resistance_hover_shows_the_recorded_strengths() -> std::io::Result<()> {
    use content_reader_localisation::localisation::Argument;
    use viewer_counter_products::map::province_tooltips::{Cache, ModeHover};
    let fixture = Fixture::load("viewer-resistance-hover")?;
    let document = &fixture.document;
    let viewer_playback::map_source::Current::Frame(frame) = document.playback.current() else {
        panic!("the fixture hour is a gap");
    };
    let resistance = frame
        .complete::<replay_data::rows::tables::StateResistance>()
        .map_err(std::io::Error::other)?
        .iter()
        .find(|row| {
            row.active != 0
                && std::env::var("BATTLEPLAN_RESISTANCE_STATE").or_else(|_| std::env::var("HOI4_RESISTANCE_STATE"))
                    .ok()
                    .is_none_or(|state| state.parse::<u32>().ok() == Some(row.state))
        })
        .expect("an occupied state with resistance")
        .clone();
    let province = frame
        .complete::<replay_data::rows::tables::Provinces>()
        .map_err(std::io::Error::other)?
        .iter()
        .find(|province| province.state == resistance.state)
        .expect("a province of the occupied state")
        .id;
    let vision = document.vision(&Default::default())?;
    let hover = Cache::default().request(
        &document.counter_content,
        document.icons.raids.movement_reads(),
        frame,
        province,
        &vision,
        false,
        &Arc::new(Vec::new()),
        &document.playback.roster_sampler(),
        &ModeHover::Occupation,
        &[],
        || {},
    );
    let loc = &document.counter_content.localisation;
    let strength = |key: &str, value: i64| {
        loc.localise_with(key.as_bytes(), &[("TOT", Argument::Fixed5(value / 100))])
            .unwrap()
    };
    assert!(
        hover
            .title
            .contains(&strength("RESISTANCE_STRENGTH", resistance.level)),
        "{}
{:?}",
        hover.title,
        hover.refused
    );
    assert!(
        hover
            .title
            .contains(&strength("COMPLIANCE_STRENGTH", resistance.compliance)),
        "{}",
        hover.title
    );
    assert!(
        hover.refused.is_empty(),
        "{}\n{:?}",
        hover.title,
        hover.refused
    );
    assert!(!hover.title.contains("\n\n\n"), "{}", hover.title);
    std::fs::write(
        fixture.output.join("hover.txt"),
        format!(
            "state={} province={} hour={}\n{}\n",
            resistance.state, province, frame.hour, hover.title
        ),
    )?;
    Ok(())
}
