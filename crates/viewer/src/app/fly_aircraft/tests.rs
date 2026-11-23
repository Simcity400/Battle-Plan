use super::*;
use std::{collections::BTreeMap, time::Duration};

fn assets(name: &str) -> Arc<AircraftAssets> {
    content_reader_script::test_game::enter();
    let root =
        std::env::temp_dir().join(format!("viewer-fly-aircraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut mesh = b"@@b@[object\0[[frame\0[[[mesh\0".to_vec();
    for (name, values) in [
        ("p", [0_f32, 0., 0., 1., 0., 0., 0., 1., 0.]),
        ("n", [0., 1., 0., 0., 1., 0., 0., 1., 0.]),
    ] {
        mesh.extend([b'!', name.len() as u8]);
        mesh.extend(name.as_bytes());
        mesh.push(b'f');
        mesh.extend((values.len() as u32).to_le_bytes());
        mesh.extend(values.into_iter().flat_map(f32::to_le_bytes));
    }
    mesh.extend(b"!\x03trii\x03\0\0\0\0\0\0\0\x01\0\0\0\x02\0\0\0");
    mesh.extend(b"[[[[material\0!\x06shaders\x01\0\0\0\x0e\0\0\0AuthoredShader");
    for (path, bytes) in [
        (
            "gfx/models/planes.gfx",
            br#"objectTypes = { pdxmesh = { name = "plane_mesh" file = "gfx/models/plane.mesh" } }"#
                .as_slice(),
        ),
        (
            "gfx/entities/planes.asset",
            br#"entity = { name = "plane_entity" pdxmesh = "plane_mesh" }
                entity = { name = "ghost_entity" }"#
                .as_slice(),
        ),
        ("gfx/models/plane.mesh", mesh.as_slice()),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let map = content_reader_map::map::MapData {
        width: 1,
        height: 1,
        province_ids: vec![0],
        definitions: Default::default(),
        country_colors: Default::default(),
    };
    let assets = AircraftAssets::load(
        &layers,
        content_reader_image::native::Decode::default(),
        &map,
        (
            &content_reader_defines::defines::Defines::new().unwrap(),
            &content_reader_defines::defines::Defines::new().unwrap(),
        ),
        &BTreeMap::new(),
        &mut Vec::new(),
    )
    .unwrap();
    Arc::new(assets)
}

fn settle(fly: &mut FlyAircraft, assets: &Arc<AircraftAssets>) -> FlyAircraftStatus {
    let (send, receive) = mpsc::channel();
    let mut last = None;
    for _ in 0..100 {
        let repaint = send.clone();
        if let Some(status) = fly.poll(Some(assets), move || {
            let _ = repaint.send(());
        }) {
            last = Some(status);
        }
        match &last {
            Some(status) if status.state != FlyAircraftState::Loading => {
                return status.clone();
            }
            _ => {
                let _ = receive.recv_timeout(Duration::from_millis(100));
            }
        }
    }
    panic!("the aircraft never finished loading: {last:?}");
}

fn status(aircraft: Option<&str>, state: FlyAircraftState) -> FlyAircraftStatus {
    FlyAircraftStatus {
        aircraft: aircraft.map(str::to_owned),
        state,
    }
}

#[test]
fn a_chosen_aircraft_reports_loading_then_ready_and_bakes_again_for_a_new_map() {
    let first = assets("first");
    let mut fly = FlyAircraft::default();
    assert_eq!(
        fly.poll(Some(&first), || {}),
        Some(status(None, FlyAircraftState::Ready))
    );
    assert_eq!(fly.poll(Some(&first), || {}), None);
    assert!(fly.ready(&first).is_none());
    fly.choose(Some("plane_entity".into()));
    assert_eq!(
        fly.poll(None, || {}),
        Some(status(Some("plane_entity"), FlyAircraftState::Loading))
    );
    assert_eq!(
        settle(&mut fly, &first),
        status(Some("plane_entity"), FlyAircraftState::Ready)
    );
    let baked = fly.ready(&first).unwrap().clone();
    assert_eq!(baked.name, "plane_entity");
    assert_eq!(first.library().slot("plane_entity"), Some(baked.slot));
    let second = assets("second");
    assert_eq!(
        fly.poll(Some(&second), || {}),
        Some(status(Some("plane_entity"), FlyAircraftState::Loading))
    );
    assert!(fly.ready(&second).is_none());
    assert_eq!(
        settle(&mut fly, &second),
        status(Some("plane_entity"), FlyAircraftState::Ready)
    );
    assert_eq!(
        second.library().slot("plane_entity"),
        Some(fly.ready(&second).unwrap().slot)
    );
    assert!(fly.ready(&first).is_none());
    fly.choose(None);
    assert_eq!(
        fly.poll(Some(&second), || {}),
        Some(status(None, FlyAircraftState::Ready))
    );
    assert!(fly.ready(&second).is_none());
}

#[test]
fn a_refused_aircraft_reports_why_and_is_never_ready() {
    let assets = assets("refused");
    let mut fly = FlyAircraft::default();
    for (name, reason) in [
        ("ghost_entity", "it names no pdxmesh"),
        ("no_such_plane", "no descriptor registers it"),
    ] {
        fly.choose(Some(name.into()));
        assert_eq!(
            settle(&mut fly, &assets),
            status(
                Some(name),
                FlyAircraftState::Refused {
                    reason: reason.into()
                }
            )
        );
        assert!(fly.ready(&assets).is_none());
    }
}

#[test]
fn fly_mode_draws_the_chosen_plane_and_no_aircraft_without_a_choice() {
    use crate::app::fly::placed;
    use viewer_models::ModelInstance;
    let assets = assets("scene");
    let view = ModelInstance {
        rotation: 1.5,
        ..ModelInstance::default()
    };
    let mut fly = FlyAircraft::default();
    fly.poll(Some(&assets), || {});
    assert_eq!(
        placed(fly.ready(&assets).map(|plane| plane.slot), Some(view)),
        None
    );
    fly.choose(Some("plane_entity".into()));
    settle(&mut fly, &assets);
    let slot = assets.library().slot("plane_entity").unwrap();
    assert_eq!(
        placed(fly.ready(&assets).map(|plane| plane.slot), Some(view)),
        Some((slot, view))
    );
    assert_eq!(assets.library().meshes[slot].name, "plane_entity");
    fly.choose(None);
    fly.poll(Some(&assets), || {});
    assert_eq!(
        placed(fly.ready(&assets).map(|plane| plane.slot), Some(view)),
        None
    );
}
